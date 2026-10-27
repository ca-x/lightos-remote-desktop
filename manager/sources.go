package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"regexp"
	"strings"
	"time"
)

// Sources publish authorized entry URLs. The manager owns neither their
// installation state nor their connection/authentication implementation.
type desktopSource struct {
	AppID string `json:"app_id"`
	Name  string `json:"name"`
	Path  string `json:"path"`
}

var appIDPattern = regexp.MustCompile(`^[a-zA-Z0-9][a-zA-Z0-9.-]{0,127}$`)

func (s desktopSource) validate() error {
	if !appIDPattern.MatchString(s.AppID) || !strings.HasPrefix(s.Path, "/") || strings.HasPrefix(s.Path, "//") || strings.ContainsAny(s.Path, "?#\\\r\n") {
		return errors.New("invalid desktop source")
	}
	return nil
}

func delegatedGET(ctx context.Context, raw string, headers http.Header, output any) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, raw, nil)
	if err != nil {
		return err
	}
	ticket := headers.Get("X-HC-User-Ticket")
	if ticket == "" {
		return errors.New("user authorization ticket is unavailable")
	}
	req.Header.Set("X-HC-User-Ticket", ticket)
	req.Header.Set("Accept", "application/json")
	client := &http.Client{Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	res, err := client.Do(req)
	if err != nil {
		return errors.New("source connection failed")
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return fmt.Errorf("source returned HTTP %d", res.StatusCode)
	}
	return json.NewDecoder(io.LimitReader(res.Body, 4<<20)).Decode(output)
}

func (s desktopSource) targets(ctx context.Context, headers http.Header) ([]desktopTarget, error) {
	var domain struct {
		Domain string `json:"Domain"`
		Error  string `json:"Error"`
	}
	if err := delegatedGET(ctx, "http://app.home.system.lzcx/sys/api/queryAppDomain?appid="+url.QueryEscape(s.AppID), headers, &domain); err != nil {
		return nil, err
	}
	if domain.Error != "" || domain.Domain == "" {
		return nil, nil
	}
	base, err := url.Parse("https://" + domain.Domain)
	if err != nil || base.Hostname() == "" || base.User != nil || base.Path != "" || base.RawQuery != "" || base.Fragment != "" {
		return nil, errors.New("invalid source domain")
	}
	var items []struct {
		Name       string `json:"name"`
		Platform   string `json:"platform"`
		Distro     string `json:"distro"`
		URL        string `json:"url"`
		DesktopURL string `json:"remote_desktop_url"`
		Enabled    *bool  `json:"remote_desktop_enabled"`
		Status     string `json:"status"`
	}
	if err := delegatedGET(ctx, "http://app."+s.AppID+".lzcx"+s.Path, headers, &items); err != nil {
		return nil, err
	}
	targets := make([]desktopTarget, 0)
	for _, item := range items {
		if item.Enabled != nil && !*item.Enabled {
			continue
		}
		if item.Status != "" && item.Status != "running" && item.Status != "online" {
			continue
		}
		entry := item.URL
		if entry == "" {
			entry = item.DesktopURL
		}
		if entry == "" {
			continue
		}
		parsed, err := url.Parse(entry)
		if err != nil || parsed.User != nil {
			continue
		}
		resolved := base.ResolveReference(parsed)
		if resolved.Scheme != "https" || resolved.Host != base.Host {
			continue
		}
		sum := sha256.Sum256([]byte(s.AppID + "\x00" + resolved.String()))
		platform := item.Platform
		if platform == "" {
			platform = item.Distro
		}
		targets = append(targets, desktopTarget{ID: hex.EncodeToString(sum[:16]), Name: item.Name, Platform: platform, URL: resolved.String()})
	}
	return targets, nil
}
