package main

import (
	"net/http"
	"time"
)

// Keyboard capture belongs to the authenticated viewing device, independently
// of whether the selected target is a physical machine or a container desktop.
func (s *server) viewerInput(w http.ResponseWriter, r *http.Request, targetID string) {
	uid, device := owner(r), currentDeviceID(r)
	if uid == "" || device == "" {
		writeJSON(w, http.StatusOK, map[string]any{"available": false, "reason": "Viewing-device identity is unavailable; use browser fullscreen keyboard capture"})
		return
	}
	s.hosts.mu.Lock()
	target, allowed := s.hosts.hosts[targetID]
	viewerID := ""
	for id, record := range s.hosts.hosts {
		if record.source == nil && record.owner == uid && record.registration.DeviceID == device && record.until.After(time.Now()) && record.registration.Desktop.Ready {
			viewerID = id
			break
		}
	}
	s.hosts.mu.Unlock()
	if !allowed || target.owner != uid || viewerID == "" || viewerID == targetID {
		writeJSON(w, http.StatusOK, map[string]any{"available": false, "reason": "Enable the runtime on the Current device card for native keyboard capture"})
		return
	}
	request := r.Clone(r.Context())
	request.URL.Path = "/desktop/" + viewerID + "/viewer-input"
	request.URL.RawPath = ""
	s.hosts.proxy(w, request)
}
