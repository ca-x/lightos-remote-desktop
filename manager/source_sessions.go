package main

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"net/http/httputil"
	"net/url"
	"strconv"
	"strings"
	"time"
)

func (s *server) rememberSource(uid string, source desktopSource, target desktopTarget) desktopTarget {
	id := hostID(uid, "source:"+target.ID)
	registration := hostRegistration{ID: target.ID, Name: target.Name, Platform: target.Platform}
	registration.Desktop.Enabled, registration.Desktop.Ready = target.Enabled, target.Ready
	record := hostRecord{registration: registration, owner: uid, until: time.Now().Add(10 * time.Second), source: &source, sourceURL: target.URL}
	s.hosts.mu.Lock()
	s.hosts.hosts[id] = record
	s.hosts.mu.Unlock()
	target.ID, target.URL, target.Kind = id, "/desktop/"+id+"/", "instance"
	target.Permissions = map[string]any{"screen": "granted", "input": "granted", "can_start": true, "requesting": false}
	if stamp := previewStamp(id); stamp != "" {
		target.Preview = "/api/previews/" + id + "?v=" + stamp
	}
	return target
}

func (s *server) sourceSession(r *http.Request, id string) (hostRecord, error) {
	s.hosts.mu.Lock()
	cached, exists := s.hosts.hosts[id]
	s.hosts.mu.Unlock()
	if exists && cached.owner == owner(r) && cached.source != nil && cached.until.After(time.Now()) {
		return cached, nil
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	for _, source := range s.sources {
		if exists && cached.source != nil && source.AppID != cached.source.AppID {
			continue
		}
		targets, err := source.targets(ctx, r.Header)
		if err != nil {
			continue
		}
		for _, target := range targets {
			remembered := s.rememberSource(owner(r), source, target)
			if remembered.ID == id {
				s.hosts.mu.Lock()
				record := s.hosts.hosts[id]
				s.hosts.mu.Unlock()
				return record, nil
			}
		}
	}
	return hostRecord{}, errors.New("desktop source is unavailable or no longer authorized")
}

func (s *server) desktop(w http.ResponseWriter, r *http.Request) {
	id, suffix, found := strings.Cut(strings.TrimPrefix(r.URL.Path, "/desktop/"), "/")
	if !found || !validHostID(id) {
		http.NotFound(w, r)
		return
	}
	if suffix == "viewer-input" {
		s.viewerInput(w, r, id)
		return
	}
	s.hosts.mu.Lock()
	record, exists := s.hosts.hosts[id]
	s.hosts.mu.Unlock()
	if exists && record.source == nil {
		s.hosts.proxy(w, r)
		return
	}
	record, err := s.sourceSession(r, id)
	if err != nil {
		writeFailure(w, http.StatusForbidden, "desktop_unavailable")
		return
	}
	if !record.registration.Desktop.Ready {
		writeFailure(w, http.StatusConflict, "desktop_offline")
		return
	}
	upstream, err := url.Parse(record.sourceURL)
	if err != nil || r.Header.Get("X-HC-User-Ticket") == "" {
		writeFailure(w, 403, "source_authorization_required")
		return
	}
	if origin := r.Header.Get("Origin"); origin != "" && origin != "https://"+r.Host && origin != "http://"+r.Host {
		writeFailure(w, 403, "invalid_desktop_origin")
		return
	}
	internal := "app." + record.source.AppID + ".lzcx"
	ticket := r.Header.Get("X-HC-User-Ticket")
	basePath := strings.TrimRight(upstream.Path, "/") + "/"
	transport := &http.Transport{ResponseHeaderTimeout: 20 * time.Second}
	defer transport.CloseIdleConnections()
	proxy := &httputil.ReverseProxy{
		Director: func(req *http.Request) {
			req.URL.Scheme, req.URL.Host = "http", internal
			req.URL.Path, req.URL.RawPath = basePath+suffix, ""
			query := req.URL.Query()
			for key, values := range upstream.Query() {
				if _, exists := query[key]; !exists {
					query[key] = values
				}
			}
			req.URL.RawQuery = query.Encode()
			req.Host = internal
			for key := range req.Header {
				lower := strings.ToLower(key)
				if strings.HasPrefix(lower, "x-hc-") || lower == "cookie" || lower == "authorization" || lower == "lzc-auth-token" || lower == "lzc-api-auth-token" {
					req.Header.Del(key)
				}
			}
			req.Header.Set("X-HC-User-Ticket", ticket)
			req.Header.Set("X-Forwarded-Host", internal)
			if req.Header.Get("Origin") != "" {
				req.Header.Set("Origin", "http://"+internal)
			}
			// Let the transport decode gzip so HTML can be extended safely.
			req.Header.Del("Accept-Encoding")
		},
		Transport: transport, FlushInterval: -1,
		ModifyResponse: func(response *http.Response) error {
			if location := response.Header.Get("Location"); location != "" {
				resolved, err := upstream.Parse(location)
				if err == nil && (resolved.Host == upstream.Host || resolved.Host == internal) && strings.HasPrefix(resolved.Path, basePath) {
					response.Header.Set("Location", "/desktop/"+id+"/"+strings.TrimPrefix(resolved.Path, basePath))
				}
			}
			if response.StatusCode != http.StatusOK || !strings.Contains(response.Header.Get("Content-Type"), "text/html") {
				return nil
			}
			body, err := io.ReadAll(io.LimitReader(response.Body, 2<<20))
			response.Body.Close()
			if err != nil {
				return err
			}
			marker := bytes.Index(bytes.ToLower(body), []byte("</head>"))
			if marker >= 0 {
				script := []byte(`<script src="/session-preview.js"></script>`)
				expanded := make([]byte, 0, len(body)+len(script))
				expanded = append(expanded, body[:marker]...)
				expanded = append(expanded, script...)
				expanded = append(expanded, body[marker:]...)
				body = expanded
			}
			response.Body = io.NopCloser(bytes.NewReader(body))
			response.ContentLength = int64(len(body))
			response.Header.Set("Content-Length", strconv.Itoa(len(body)))
			response.Header.Del("Content-Encoding")
			response.Header.Del("ETag")
			return nil
		},
		ErrorHandler: func(w http.ResponseWriter, _ *http.Request, _ error) { writeFailure(w, 502, "desktop_unavailable") },
	}
	proxy.ServeHTTP(w, r)
}

func (s *server) preview(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	if !validHostID(id) {
		http.NotFound(w, r)
		return
	}
	s.hosts.mu.Lock()
	record, exists := s.hosts.hosts[id]
	s.hosts.mu.Unlock()
	if !exists || record.source != nil && r.Method == http.MethodPost && !record.until.After(time.Now()) {
		if _, err := s.sourceSession(r, id); err != nil {
			http.NotFound(w, r)
			return
		}
	}
	s.hosts.preview(w, r)
}
