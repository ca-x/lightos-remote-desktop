// Remote desktop management is packaged independently. LightOS remains the
// authority for target discovery and connection authorization.
package main

import (
	"context"
	"embed"
	"encoding/json"
	"errors"
	"io/fs"
	"log"
	"net/http"
	"net/url"
	"os/exec"
	"strings"
	"time"
)

//go:embed web/*
var webAssets embed.FS

func adminURL(ctx context.Context) (*url.URL, error) {
	output, err := exec.CommandContext(ctx, "/lzcinit/lightosctl", "system", "admin-info", "--json").Output()
	if err != nil {
		return nil, errors.New("LightOS is unavailable; install or start LightOS first")
	}
	var info struct {
		BaseURL string `json:"base_url"`
	}
	if json.Unmarshal(output, &info) != nil {
		return nil, errors.New("invalid LightOS discovery response")
	}
	base, err := url.Parse(info.BaseURL)
	if err != nil || base.Scheme != "https" || base.Host == "" || base.User != nil || base.RawQuery != "" || base.Fragment != "" {
		return nil, errors.New("verified HTTPS LightOS endpoint required")
	}
	return base, nil
}

func targets(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	if r.Method != http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if strings.TrimSpace(r.Header.Get("X-HC-User-ID")) == "" {
		http.Error(w, "Login required", http.StatusUnauthorized)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 25*time.Second)
	defer cancel()
	base, err := adminURL(ctx)
	if err != nil {
		http.Error(w, err.Error(), http.StatusServiceUnavailable)
		return
	}
	endpoint := base.ResolveReference(&url.URL{Path: "/api/remote-desktop/targets"})
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint.String(), nil)
	if err != nil {
		http.Error(w, "LightOS request failed", http.StatusBadGateway)
		return
	}
	// The LightOS ingress verifies the user's actual session. No owner/admin
	// headers are fabricated and no application-wide credential substitutes it.
	for _, key := range []string{"Cookie", "Authorization", "Accept-Language"} {
		if value := r.Header.Get(key); value != "" {
			req.Header.Set(key, value)
		}
	}
	client := &http.Client{Timeout: 24 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	res, err := client.Do(req)
	if err != nil {
		http.Error(w, "Cannot reach LightOS", http.StatusBadGateway)
		return
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		http.Error(w, "LightOS rejected the request; check login, access and the installed LightOS version", res.StatusCode)
		return
	}
	var data struct {
		Targets []struct {
			ID              string `json:"id"`
			Name            string `json:"name"`
			Kind            string `json:"kind"`
			Platform        string `json:"platform"`
			URL             string `json:"url"`
			RequiresConsent bool   `json:"requires_consent,omitempty"`
		} `json:"targets"`
		Unavailable []string `json:"unavailable"`
		LightOSURL  string   `json:"lightos_url"`
	}
	if json.NewDecoder(http.MaxBytesReader(w, res.Body, 4<<20)).Decode(&data) != nil {
		http.Error(w, "Invalid LightOS response", http.StatusBadGateway)
		return
	}
	for i := range data.Targets {
		path, err := url.Parse(data.Targets[i].URL)
		if err != nil || path.IsAbs() || path.Host != "" || !strings.HasPrefix(path.Path, "/remote-desktop/") {
			http.Error(w, "Invalid desktop endpoint", http.StatusBadGateway)
			return
		}
		data.Targets[i].URL = base.ResolveReference(path).String()
	}
	data.LightOSURL = base.String()
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	_ = json.NewEncoder(w).Encode(data)
}

func main() {
	web, err := fs.Sub(webAssets, "web")
	if err != nil {
		log.Fatal(err)
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/api/remote-desktop/targets", targets)
	files := http.FileServer(http.FS(web))
	mux.HandleFunc("/", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-cache")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		w.Header().Set("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'self'")
		files.ServeHTTP(w, r)
	})
	server := &http.Server{Addr: ":8080", Handler: mux, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 60 * time.Second}
	log.Fatal(server.ListenAndServe())
}
