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
	ID              string `json:"id"`
	Name            string `json:"name"`
	Platform        string `json:"platform,omitempty"`
	URL             string `json:"url"`
	RequiresConsent bool   `json:"requires_consent,omitempty"`
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

type server struct {
	hosts   *hostRegistry
	sources []desktopSource
}

func (s *server) targets(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	targets := s.hosts.list(owner(r))
	warnings := make([]string, 0)
	for _, source := range s.sources {
		items, err := source.targets(ctx, r.Header)
		if err != nil {
			warnings = append(warnings, source.Name)
			log.Printf("desktop source %s: %v", source.AppID, err)
			continue
		}
		targets = append(targets, items...)
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
	s := &server{hosts: newHostRegistry(), sources: sources}
	mux := http.NewServeMux()
	mux.HandleFunc("/api/targets", s.targets)
	mux.HandleFunc("/api/hosts", s.hosts.register)
	mux.HandleFunc("/desktop/", s.hosts.proxy)
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
