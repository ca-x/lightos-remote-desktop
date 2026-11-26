package main

import (
	"context"
	"time"
)

// The presence of a request is saved. Confirmation belongs to one live
// controller registration and must be obtained again after it changes.
type desktopStop struct {
	confirmedService string
	retryAt          time.Time
	running          bool
}

func (record hostRecord) sameDevice(other hostRecord) bool {
	return record.owner == other.owner && record.registration.DeviceID == other.registration.DeviceID &&
		record.registration.BoxID == other.registration.BoxID
}

func (record hostRecord) desktopReady() bool {
	return record.stop == nil && record.registration.Desktop.Ready
}

func (record hostRecord) stopStatus() string {
	if record.stop.confirmedService == record.registration.ControlService && !record.registration.Desktop.Enabled {
		return "disabled"
	}
	protocol, _ := record.registration.Agent["controller_protocol"].(float64)
	if record.stop.running || (record.stop.retryAt.IsZero() && protocol >= 2) {
		return "stopping"
	}
	return "stop_pending"
}

// The registry must be locked when changing or scheduling a desktop command.
func (h *hostRegistry) requestDesktopStop(id string, record hostRecord) error {
	if record.stop == nil {
		updated := record
		updated.stop = &desktopStop{}
		h.hosts[id] = updated
		if err := h.persist(); err != nil {
			h.hosts[id] = record
			return err
		}
		record = updated
	} else if !record.stop.running {
		*record.stop = desktopStop{}
	}
	h.scheduleDesktopStop(id, record)
	return nil
}

func (h *hostRegistry) beginDesktopStart(id string, record hostRecord) *controlFailure {
	if record.starting || (record.stop != nil && record.stop.running) {
		return &controlFailure{409, "desktop_control_busy"}
	}
	updated := record
	updated.starting = true
	updated.stop = nil
	h.hosts[id] = updated
	if record.stop != nil {
		if err := h.persist(); err != nil {
			h.hosts[id] = record
			return &controlFailure{500, "desktop_preference_save_failed"}
		}
	}
	return nil
}

func (h *hostRegistry) finishDesktopStart(id string, started hostRecord) {
	h.mu.Lock()
	defer h.mu.Unlock()
	if record, ok := h.hosts[id]; ok && record.sameDevice(started) {
		record.starting = false
		h.hosts[id] = record
		h.scheduleDesktopStop(id, record)
	}
}

func (h *hostRegistry) scheduleDesktopStop(id string, record hostRecord) {
	stop := record.stop
	if stop == nil || stop.running || record.starting || !record.until.After(time.Now()) || time.Now().Before(stop.retryAt) || record.stopStatus() == "disabled" {
		return
	}
	protocol, _ := record.registration.Agent["controller_protocol"].(float64)
	if protocol < 2 || record.registration.ControlService == "" {
		return
	}
	stop.running = true
	// The controller can publish to /api/hosts before acknowledging /stop.
	// Forward the command independently of the registration request.
	go h.stopDesktop(id, record)
}

func (h *hostRegistry) stopDesktop(id string, attempted hostRecord) {
	reply, failure := forwardHostControl(context.Background(), attempted, "stop")
	state, _ := reply["state"].(map[string]any)
	accepted, _ := reply["accepted"].(bool)
	enabled, hasEnabled := state["enabled"].(bool)
	ready, hasReady := state["ready"].(bool)

	h.mu.Lock()
	defer h.mu.Unlock()
	current := h.hosts[id]
	stop := attempted.stop
	stop.running = false
	if current.stop != stop {
		return
	}
	if current.registration.ControlService != attempted.registration.ControlService {
		h.scheduleDesktopStop(id, current)
		return
	}
	if failure != nil || !accepted || !hasEnabled || enabled || !hasReady || ready {
		stop.retryAt = time.Now().Add(5 * time.Second)
		return
	}
	stop.confirmedService = current.registration.ControlService
	stop.retryAt = time.Time{}
	current.registration.Desktop.Enabled = false
	current.registration.Desktop.Ready = false
	current.registration.Agent = state
	h.hosts[id] = current
}
