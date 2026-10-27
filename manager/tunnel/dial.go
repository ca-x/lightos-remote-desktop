package tunnel

import (
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"encoding/base64"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/gorilla/websocket"
)

const ticketHeader = "X-Lightos-Client-Publish-Ticket"
const protocol = "lightos-client-publish.v1"

type Target struct {
	InstanceID  string
	AccountID   string
	BoxID       string
	DeviceID    string
	Epoch       string
	Secret      string
	Service     string
	DeviceURL   string
	DeviceToken string
}

func ValidHost(value string) (string, bool) {
	host := strings.TrimSpace(value)
	if host == "" || len(host) > 255 || strings.ContainsAny(host, "\x00\r\n/?#@") {
		return "", false
	}
	return host, true
}

func Dial(ctx context.Context, target Target, publishID, host string, port int) (net.Conn, error) {
	host, ok := ValidHost(host)
	if !ok || port < 1 || port > 65535 || strings.TrimSpace(publishID) == "" ||
		len(publishID) > 128 || strings.ContainsAny(publishID, "\r\n") {
		return nil, errors.New("invalid physical service target")
	}
	for _, value := range []string{target.InstanceID, target.AccountID, target.BoxID, target.DeviceID,
		target.Epoch, target.Secret, target.Service, target.DeviceToken} {
		if strings.TrimSpace(value) == "" || strings.ContainsAny(value, "\r\n") {
			return nil, errors.New("physical service authorization is unavailable")
		}
	}
	if strings.ContainsAny(target.Service, "/?#\\") {
		return nil, errors.New("invalid physical service registration")
	}
	base, err := url.Parse(target.DeviceURL)
	if err != nil || base.Scheme != "https" || base.Host == "" || base.User != nil || base.RawQuery != "" || base.Fragment != "" {
		return nil, errors.New("verified device endpoint is required")
	}
	base.Scheme = "wss"
	base.Path = strings.TrimRight(base.Path, "/") + "/s/" + target.Service + "/publish/connect"
	base.RawPath = ""
	nonce := make([]byte, 32)
	if _, err := rand.Read(nonce); err != nil {
		return nil, err
	}
	payload := strings.Join([]string{"lightos-client-publish-v1", target.InstanceID, target.AccountID,
		target.BoxID, target.DeviceID, target.Epoch, publishID, host, strconv.Itoa(port),
		strconv.FormatInt(time.Now().Add(60*time.Second).Unix(), 10), base64.RawURLEncoding.EncodeToString(nonce)}, "\n")
	mac := hmac.New(sha256.New, []byte(target.Secret))
	_, _ = mac.Write([]byte(payload))
	ticket := base64.RawURLEncoding.EncodeToString([]byte(payload)) + "." + base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	head := http.Header{}
	head.Set("lzc_dapi_auth_token", target.DeviceToken)
	head.Set(ticketHeader, ticket)
	dialer := websocket.Dialer{Subprotocols: []string{protocol}, HandshakeTimeout: 12 * time.Second,
		TLSClientConfig: &tls.Config{MinVersion: tls.VersionTLS12}}
	ws, response, err := dialer.DialContext(ctx, base.String(), head)
	if err != nil {
		if response != nil {
			defer response.Body.Close()
			return nil, fmt.Errorf("physical service tunnel returned HTTP %d: %w", response.StatusCode, err)
		}
		return nil, err
	}
	if ws.Subprotocol() != protocol {
		ws.Close()
		return nil, errors.New("physical service tunnel protocol mismatch")
	}
	ws.SetReadLimit(64 << 10)
	return &streamConn{ws: ws}, nil
}
