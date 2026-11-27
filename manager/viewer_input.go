package main

import (
	"net/http"
	"time"

	"github.com/gorilla/websocket"
)

// Keyboard capture belongs to the authenticated viewing device, independently
// of whether the selected target is a physical machine or a container desktop.
func (s *server) viewerInput(w http.ResponseWriter, r *http.Request, targetID string) {
	uid, device := owner(r), currentDeviceID(r)
	if uid == "" || device == "" {
		writeJSON(w, http.StatusOK, map[string]any{"available": false, "reason": "Viewing-device identity is unavailable; use browser fullscreen keyboard capture"})
		return
	}
	if origin := r.Header.Get("Origin"); r.Header.Get("Sec-Fetch-Site") == "cross-site" || origin != "" && origin != "https://"+r.Host && origin != "http://"+r.Host {
		writeFailure(w, http.StatusForbidden, "invalid_desktop_origin")
		return
	}
	s.hosts.mu.Lock()
	target, allowed := s.hosts.hosts[targetID]
	var viewer hostRecord
	viewerID := ""
	for id, record := range s.hosts.hosts {
		if record.source == nil && record.owner == uid && record.registration.DeviceID == device && record.until.After(time.Now()) && (record.desktopReady() || record.canLaunch()) {
			viewer, viewerID = record, id
			break
		}
	}
	if !allowed || target.owner != uid || target.source == nil && !target.until.After(time.Now()) || !target.desktopReady() || viewerID == "" || viewerID == targetID {
		s.hosts.mu.Unlock()
		writeJSON(w, http.StatusOK, map[string]any{"available": false, "reason": "Viewing-device keyboard capture is unavailable; use browser fullscreen keyboard capture"})
		return
	}
	// Opening a remote window probes keyboard support. Leave the local host
	// idle until the viewer actually requests native capture via websocket.
	if !viewer.desktopReady() && !websocket.IsWebSocketUpgrade(r) {
		s.hosts.mu.Unlock()
		// The lease may be requested on demand. Its handshake checks the
		// installed helper, including on older runtime versions.
		writeJSON(w, http.StatusOK, map[string]bool{"available": true})
		return
	}
	if !viewer.desktopReady() {
		start, err := s.hosts.prepareDesktopLaunch(viewerID, viewer)
		s.hosts.mu.Unlock()
		if err != nil {
			writeFailure(w, 500, "desktop_preference_save_failed")
			return
		}
		if start {
			_, failure := forwardHostControl(r.Context(), viewer, "start")
			s.hosts.finishDesktopStart(viewerID, viewer)
			if failure != nil {
				writeFailure(w, failure.status, failure.code)
				return
			}
		}
	} else {
		s.hosts.mu.Unlock()
	}
	request := r.Clone(r.Context())
	request.URL.Path = "/desktop/" + viewerID + "/viewer-input"
	request.URL.RawPath = ""
	s.hosts.proxy(w, request)
}
