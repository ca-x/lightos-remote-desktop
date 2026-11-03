package main

import (
	"context"
	"embed"
	"encoding/json"
	"io/fs"
	"log"
	"net/http"
	"os"
	"sort"
	"strings"
	"time"
)

//go:embed web/*
var webAssets embed.FS

type desktopTarget struct {
	RuntimeVersion     string         `json:"runtime_version,omitempty"`
	AvailableVersion   string         `json:"available_version,omitempty"`
	Installed          bool           `json:"installed"`
	CanInstall         bool           `json:"can_install"`
	RuntimeSupported   bool           `json:"runtime_supported"`
	ControllerProtocol int            `json:"controller_protocol,omitempty"`
	UpdateAvailable    bool           `json:"update_available"`
	Deployment         map[string]any `json:"deployment,omitempty"`
	RuntimeError       string         `json:"runtime_error,omitempty"`
	agent              map[string]any

	Kind            string         `json:"kind"`
	CurrentDevice   bool           `json:"current_device,omitempty"`
	ID              string         `json:"id"`
	Name            string         `json:"name"`
	Platform        string         `json:"platform,omitempty"`
	URL             string         `json:"url"`
	RequiresConsent bool           `json:"requires_consent,omitempty"`
	Managed         bool           `json:"managed"`
	Online          bool           `json:"online"`
	Enabled         bool           `json:"enabled"`
	Ready           bool           `json:"ready"`
	Status          string         `json:"status,omitempty"`
	Permissions     map[string]any `json:"permissions,omitempty"`
	Preview         string         `json:"preview,omitempty"`
}

func writeJSON(w http.ResponseWriter, status int, data any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(data)
}
func writeFailure(w http.ResponseWriter, status int, code string) {
	writeJSON(w, status, map[string]string{"error": code})
}
func owner(r *http.Request) string { return strings.TrimSpace(r.Header.Get("X-HC-User-ID")) }

// The application gateway supplies the same stable device ID used by the
// platform SDK's current-session device lookup. It is not a hostname or peer ID.
func currentDeviceID(r *http.Request) string {
	return strings.TrimSpace(r.Header.Get("X-HC-Device-ID"))
}

type server struct {
	hosts    *hostRegistry
	sources  []desktopSource
	runtimes *runtimeCatalog
}

func (s *server) targets(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	targets := s.hosts.list(owner(r), currentDeviceID(r))
	warnings := make([]string, 0)
	for _, source := range s.sources {
		items, err := source.targets(ctx, r.Header)
		if err != nil {
			warnings = append(warnings, source.Name)
			log.Printf("desktop source %s: %v", source.AppID, err)
			continue
		}
		for _, item := range items {
			targets = append(targets, s.rememberSource(owner(r), source, item))
		}
	}
	for index := range targets {
		s.runtimes.decorate(&targets[index])
	}
	sort.Slice(targets, func(i, j int) bool { return targets[i].Name < targets[j].Name })
	writeJSON(w, http.StatusOK, map[string]any{"targets": targets, "unavailable": warnings})
}
func main() {
	web, err := fs.Sub(webAssets, "web")
	if err != nil {
		log.Fatal(err)
	}
	var sources []desktopSource
	if value := os.Getenv("REMOTE_DESKTOP_SOURCES"); value != "" {
		if err := json.Unmarshal([]byte(value), &sources); err != nil {
			log.Fatal("invalid desktop source configuration")
		}
		for _, source := range sources {
			if err := source.validate(); err != nil {
				log.Fatal(err)
			}
		}
	}
	runtimes, err := loadRuntimeCatalog()
	if err != nil {
		log.Fatal("Unable to load the bundled runtime catalog: ", err)
	}
	s := &server{hosts: newHostRegistry(), sources: sources, runtimes: runtimes}
	mux := http.NewServeMux()
	mux.HandleFunc("/api/targets", s.targets)
	mux.HandleFunc("/api/runtimes/{target}/{file}", runtimes.serve)
	mux.HandleFunc("/api/hosts", s.hosts.register)
	mux.HandleFunc("/api/hosts/{id}/{action}", s.hosts.control)
	mux.HandleFunc("/api/previews/{id}", s.preview)
	mux.HandleFunc("/desktop/", s.desktop)
	mux.Handle("/", http.FileServer(http.FS(web)))
	handler := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store")
		w.Header().Set("X-Content-Type-Options", "nosniff")
		if !strings.HasPrefix(r.URL.Path, "/desktop/") {
			w.Header().Set("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'self'")
		}
		if owner(r) == "" {
			writeFailure(w, http.StatusUnauthorized, "login_required")
			return
		}
		mux.ServeHTTP(w, r)
	})
	addr := os.Getenv("REMOTE_DESKTOP_LISTEN")
	if addr == "" {
		addr = ":8080"
	}
	log.Fatal((&http.Server{Addr: addr, Handler: handler, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 60 * time.Second}).ListenAndServe())
}
