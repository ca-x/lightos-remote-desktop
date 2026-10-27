package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"strings"
	"sync"
	"time"

	sdk "gitee.com/linakesi/lzc-sdk/lang/go"
	"gitee.com/linakesi/lzc-sdk/lang/go/common"
	"github.com/ponzS/lightos-remote-desktop/manager/tunnel"
	"google.golang.org/grpc/metadata"
)

type hostRegistration struct {
	ID             string `json:"id"`
	Name           string `json:"name"`
	Platform       string `json:"platform"`
	DeviceID       string `json:"device_id"`
	Service        string `json:"terminal_service_name"`
	DesktopService string `json:"desktop_service_name"`
	Epoch          string `json:"terminal_epoch"`
	Secret         string `json:"terminal_secret"`
	BoxID          string `json:"box_id"`
	Desktop        struct {
		Enabled         bool   `json:"enabled"`
		Ready           bool   `json:"ready"`
		Port            int    `json:"port"`
		Password        string `json:"password"`
		Epoch           string `json:"epoch"`
		RequiresConsent bool   `json:"requires_consent"`
	} `json:"desktop"`
}
type hostRecord struct {
	registration hostRegistration
	owner        string
	until        time.Time
}
type hostRegistry struct {
	mu    sync.Mutex
	hosts map[string]hostRecord
}

func newHostRegistry() *hostRegistry { return &hostRegistry{hosts: make(map[string]hostRecord)} }
func hostID(owner, id string) string {
	sum := sha256.Sum256([]byte(owner + "\x00" + id))
	return hex.EncodeToString(sum[:16])
}
func (h *hostRegistry) list(owner string) []desktopTarget {
	h.mu.Lock()
	defer h.mu.Unlock()
	targets := make([]desktopTarget, 0)
	for id, record := range h.hosts {
		if !record.until.After(time.Now()) {
			delete(h.hosts, id)
			continue
		}
		if record.owner != owner {
			continue
		}
		r := record.registration
		targets = append(targets, desktopTarget{ID: id, Name: r.Name, Platform: r.Platform, URL: "/desktop/" + id + "/", RequiresConsent: r.Desktop.RequiresConsent})
	}
	return targets
}

func ownedDevice(ctx context.Context, uid, id string) (*sdk.APIGateway, *common.EndDevice, error) {
	api, err := sdk.NewAPIGateway(ctx)
	if err != nil {
		return nil, nil, err
	}
	ctx = metadata.NewOutgoingContext(ctx, metadata.Pairs("X-Hc-User-Id", uid, "X_LZCAPI_UID", uid))
	devices, err := api.Devices.ListEndDevices(ctx, &common.ListEndDeviceRequest{Uid: uid})
	if err == nil {
		for _, d := range devices.GetDevices() {
			// ShellCoreInfo.id is the client's network peer ID. Resolve it only
			// within this user's verified devices, then store the platform ID.
			matches := d.GetUniqueDeivceId() == id || d.GetAvoidUseThisPeerId() == id
			if matches && d.GetUniqueDeivceId() != "" && d.GetIsOnline() && strings.HasPrefix(d.GetDeviceApiUrl(), "https://") {
				return api, d, nil
			}
		}
	}
	api.Close()
	return nil, nil, errors.New("device is not online or not owned by this account")
}

func (h *hostRegistry) register(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if !strings.HasPrefix(r.Header.Get("Content-Type"), "application/json") || r.Header.Get("Sec-Fetch-Site") == "cross-site" {
		writeFailure(w, http.StatusForbidden, "invalid_registration_origin")
		return
	}
	var registration hostRegistration
	decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 8192))
	if decoder.Decode(&registration) != nil || registration.ID == "" || len(registration.ID) > 128 {
		writeFailure(w, 400, "invalid_registration")
		return
	}
	id := hostID(owner(r), registration.ID)
	if !registration.Desktop.Enabled || !registration.Desktop.Ready {
		h.mu.Lock()
		// Late cleanup from a previous runtime must not withdraw its replacement.
		if current, ok := h.hosts[id]; ok && (registration.DesktopService == "" || current.registration.DesktopService == registration.DesktopService) {
			delete(h.hosts, id)
		}
		h.mu.Unlock()
		w.WriteHeader(204)
		return
	}
	d := registration.Desktop
	direct := registration.DesktopService != ""
	if direct {
		if registration.DesktopService != "cloud.lazycat.remote-desktop.managed."+registration.ID+"."+d.Epoch || d.Epoch == "" || strings.ContainsAny(registration.DesktopService, "/\\\r\n?#") {
			writeFailure(w, 400, "invalid_registration")
			return
		}
	} else if registration.Service != "cloud.lazycat.lightos.client-terminal.managed."+registration.ID+"."+registration.Epoch || d.Epoch != registration.Epoch || len(registration.Secret) < 32 || len(registration.Secret) > 256 {
		writeFailure(w, 400, "invalid_registration")
		return
	}
	if d.Port < 1 || d.Port > 65535 || len(d.Password) < 32 || len(d.Password) > 256 || len(registration.Name) > 256 {
		writeFailure(w, 400, "invalid_registration")
		return
	}
	values := []string{d.Epoch, registration.DeviceID, registration.BoxID, d.Password}
	if !direct {
		values = append(values, registration.Epoch, registration.Secret, registration.Service)
	}
	for _, value := range values {
		if value == "" || strings.ContainsAny(value, "\r\n") {
			writeFailure(w, 400, "invalid_registration")
			return
		}
	}
	ctx, cancel := context.WithTimeout(r.Context(), 6*time.Second)
	defer cancel()
	api, device, err := ownedDevice(ctx, owner(r), registration.DeviceID)
	if err != nil {
		writeFailure(w, 403, "device_unavailable")
		return
	}
	api.Close()
	registration.DeviceID = device.GetUniqueDeivceId()
	h.mu.Lock()
	h.hosts[id] = hostRecord{registration: registration, owner: owner(r), until: time.Now().Add(35 * time.Second)}
	h.mu.Unlock()
	w.WriteHeader(http.StatusNoContent)
}

func (h *hostRegistry) proxy(w http.ResponseWriter, r *http.Request) {
	if origin := r.Header.Get("Origin"); origin != "" && origin != "https://"+r.Host && origin != "http://"+r.Host {
		writeFailure(w, http.StatusForbidden, "invalid_desktop_origin")
		return
	}
	id, path, found := strings.Cut(strings.TrimPrefix(r.URL.Path, "/desktop/"), "/")
	if !found {
		http.NotFound(w, r)
		return
	}
	h.mu.Lock()
	record, ok := h.hosts[id]
	h.mu.Unlock()
	if !ok || record.owner != owner(r) || !record.until.After(time.Now()) {
		writeFailure(w, 403, "desktop_unavailable")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 6*time.Second)
	api, device, err := ownedDevice(ctx, owner(r), record.registration.DeviceID)
	if err != nil {
		cancel()
		writeFailure(w, 503, "device_unavailable")
		return
	}
	defer api.Close()
	deviceProxy, err := api.NewDeviceProxy(device.GetDeviceApiUrl())
	if err != nil {
		cancel()
		writeFailure(w, 503, "device_unavailable")
		return
	}
	token, err := deviceProxy.GetAuthToken(ctx)
	deviceProxy.Close()
	cancel()
	if err != nil {
		writeFailure(w, 503, "device_authorization_failed")
		return
	}
	host := record.registration
	transport := &http.Transport{ResponseHeaderTimeout: 15 * time.Second}
	upstream := &url.URL{Scheme: "http", Host: "physical-desktop", Path: "/"}
	if host.DesktopService != "" {
		upstream, err = url.Parse(device.GetDeviceApiUrl())
		if err != nil || upstream.Scheme != "https" || upstream.Host == "" || upstream.User != nil || upstream.RawQuery != "" || upstream.Fragment != "" {
			writeFailure(w, 503, "device_unavailable")
			return
		}
		upstream.Path = strings.TrimRight(upstream.Path, "/") + "/s/" + host.DesktopService + "/"
	} else {
		target := tunnel.Target{InstanceID: host.ID, AccountID: record.owner, BoxID: host.BoxID, DeviceID: host.DeviceID, Epoch: host.Epoch, Secret: host.Secret, Service: host.Service, DeviceURL: device.GetDeviceApiUrl(), DeviceToken: token.Token}
		transport.DialContext = func(ctx context.Context, _, _ string) (net.Conn, error) {
			return tunnel.Dial(ctx, target, "remote-desktop", "127.0.0.1", host.Desktop.Port)
		}
	}
	defer transport.CloseIdleConnections()
	proxy := &httputil.ReverseProxy{
		Director: func(req *http.Request) {
			req.URL.Scheme = upstream.Scheme
			req.URL.Host = upstream.Host
			req.URL.Path = upstream.Path + path
			req.URL.RawPath = ""
			req.Host = r.Host
			for key := range req.Header {
				lower := strings.ToLower(key)
				if strings.HasPrefix(lower, "x-hc-") || lower == "cookie" || lower == "lzc-auth-token" || lower == "lzc-api-auth-token" {
					req.Header.Del(key)
				}
			}
			if host.DesktopService != "" {
				req.Host = upstream.Host
				if req.Header.Get("Origin") != "" {
					req.Header.Set("Origin", "https://"+upstream.Host)
				}
				query := req.URL.Query()
				query.Del("lzc_dapi_auth_token")
				req.URL.RawQuery = query.Encode()
				req.Header.Set("lzc_dapi_auth_token", token.Token)
			}
			req.SetBasicAuth("lightos", host.Desktop.Password)
		}, Transport: transport, FlushInterval: -1,
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, _ error) {
			http.Error(w, "Remote desktop is unavailable", http.StatusBadGateway)
		},
	}
	proxy.ServeHTTP(w, r)
}
