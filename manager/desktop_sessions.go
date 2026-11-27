package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"time"
)

func (r hostRecord) canLaunch() bool {
	if !r.accessEnabled() || r.removal || r.packageCommand || r.switchingRuntime() ||
		(r.stop != nil && r.stopStatus() != "disabled") {
		return false
	}
	agent := r.registration.Agent
	permissions, _ := agent["permissions"].(map[string]any)
	if permissions["requesting"] == true || (!r.desktopReady() && permissions["can_start"] == false) {
		return false
	}
	if r.registration.Platform == "darwin" && (permissions["screen"] != "granted" || permissions["input"] != "granted") {
		return false
	}
	return true
}

// A desktop can have a new WebSocket before its previous one finishes closing
// (reload, reconnect, desktop switch or takeover).
type desktopSessions struct {
	connections map[*http.Request]context.CancelFunc
	timer       *time.Timer
	generation  int
}

func (h *hostRegistry) sessionsFor(id string, record hostRecord) (*desktopSessions, error) {
	if record.sessions == nil {
		record.sessions = &desktopSessions{}
		// After a manager restart the persisted lease becomes a pending stop.
		if err := h.saveDesktopState(id, record); err != nil {
			return nil, err
		}
	}
	return record.sessions, nil
}

// Called under h.mu. A launch that never reaches the desktop also expires.
func (h *hostRegistry) stopWhenIdle(id string, sessions *desktopSessions, delay time.Duration) {
	sessions.generation++
	generation := sessions.generation
	if sessions.timer != nil {
		sessions.timer.Stop()
	}
	if len(sessions.connections) != 0 {
		return
	}
	sessions.timer = time.AfterFunc(delay, func() {
		h.mu.Lock()
		defer h.mu.Unlock()
		current := h.hosts[id]
		if current.sessions != sessions || sessions.generation != generation {
			return
		}
		if current.packageCommand || current.switchingRuntime() {
			h.stopWhenIdle(id, sessions, 5*time.Second)
			return
		}
		if err := h.requestDesktopStop(id, current); err != nil {
			log.Printf("cannot save automatic desktop stop: %v", err)
			h.stopWhenIdle(id, sessions, 5*time.Second)
		}
	})
}

func (h *hostRegistry) trackConnection(id string, connected hostRecord, req *http.Request, disconnect context.CancelFunc) (func(), error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	current := h.hosts[id]
	if !current.sameDevice(connected) || current.registration.Desktop.Epoch != connected.registration.Desktop.Epoch || !current.desktopReady() || current.registration.ControlService != connected.registration.ControlService {
		return nil, errors.New("desktop registration changed")
	}
	sessions, err := h.sessionsFor(id, current)
	if err != nil {
		return nil, err
	}
	if sessions.connections == nil {
		sessions.connections = make(map[*http.Request]context.CancelFunc)
	}
	sessions.connections[req] = disconnect
	if sessions.timer != nil {
		sessions.timer.Stop()
	}
	sessions.generation++
	return func() {
		h.mu.Lock()
		defer h.mu.Unlock()
		delete(sessions.connections, req)
		if h.hosts[id].sessions == sessions {
			h.stopWhenIdle(id, sessions, 5*time.Second)
		}
	}, nil
}
