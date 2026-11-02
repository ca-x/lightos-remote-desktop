package main

import (
	"bytes"
	"encoding/hex"
	"image/jpeg"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"time"
)

func sortSavedHosts(hosts []savedHost) {
	sort.Slice(hosts, func(i, j int) bool { return hosts[i].ID < hosts[j].ID })
}
func validHostID(id string) bool {
	if len(id) != 32 {
		return false
	}
	_, err := hex.DecodeString(id)
	return err == nil
}
func previewPath(id string) string { return filepath.Join(storageDirectory(), "previews", id+".jpg") }
func previewStamp(id string) string {
	if !validHostID(id) {
		return ""
	}
	info, err := os.Stat(previewPath(id))
	if err != nil {
		return ""
	}
	return strconv.FormatInt(info.ModTime().UnixNano(), 10)
}

func (h *hostRegistry) preview(w http.ResponseWriter, r *http.Request) {
	id := r.PathValue("id")
	if !validHostID(id) {
		http.NotFound(w, r)
		return
	}
	h.mu.Lock()
	record, ok := h.hosts[id]
	h.mu.Unlock()
	if !ok || record.owner != owner(r) {
		http.NotFound(w, r)
		return
	}
	if r.Method == http.MethodGet {
		w.Header().Set("Content-Type", "image/jpeg")
		w.Header().Set("Cache-Control", "private, no-cache")
		http.ServeFile(w, r, previewPath(id))
		return
	}
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	if r.Header.Get("Sec-Fetch-Site") == "cross-site" || !strings.HasPrefix(r.Header.Get("Content-Type"), "image/jpeg") {
		writeFailure(w, 403, "invalid_preview_origin")
		return
	}
	if origin := r.Header.Get("Origin"); origin != "" && origin != "https://"+r.Host && origin != "http://"+r.Host {
		writeFailure(w, 403, "invalid_preview_origin")
		return
	}
	// A cover can only come from a live authorized desktop session.
	if !record.until.After(time.Now()) || !record.registration.Desktop.Ready {
		writeFailure(w, 409, "desktop_unavailable")
		return
	}
	data, err := io.ReadAll(http.MaxBytesReader(w, r.Body, 128<<10))
	if err != nil {
		writeFailure(w, 413, "preview_too_large")
		return
	}
	dimensions, err := jpeg.DecodeConfig(bytes.NewReader(data))
	if err != nil || dimensions.Width < 1 || dimensions.Height < 1 || dimensions.Width > 1024 || dimensions.Height > 1024 {
		writeFailure(w, 400, "invalid_preview")
		return
	}
	path := previewPath(id)
	if os.MkdirAll(filepath.Dir(path), 0700) != nil {
		writeFailure(w, 500, "preview_save_failed")
		return
	}
	temporary, err := os.CreateTemp(filepath.Dir(path), ".preview-*")
	if err != nil {
		writeFailure(w, 500, "preview_save_failed")
		return
	}
	defer os.Remove(temporary.Name())
	_, writeErr := temporary.Write(data)
	closeErr := temporary.Close()
	if writeErr != nil || closeErr != nil || os.Rename(temporary.Name(), path) != nil {
		writeFailure(w, 500, "preview_save_failed")
		return
	}
	w.WriteHeader(http.StatusNoContent)
}
