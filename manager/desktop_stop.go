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
	return (record.registration.ControlService == "" || record.accessEnabled()) && record.stop == nil && !record.removal && !record.packageCommand && !record.switchingRuntime() && record.registration.Desktop.Ready
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

// Called with the registry locked. Publish the gate and any pending runtime
// work together, so a failed save cannot leave an acknowledged partial change.
func (h *hostRegistry) saveDesktopState(id string, record hostRecord) error {
	previous := h.hosts[id]
	h.hosts[id] = record
	if err := h.persist(); err != nil {
		h.hosts[id] = previous
		return err
	}
	return nil
}

func (h *hostRegistry) setDesktopAccess(id string, record hostRecord, enabled bool) error {
	record.accessDisabled = !enabled
	if !enabled && (record.registration.Desktop.Enabled || record.sessions != nil || record.starting) {
		return h.requestDesktopStop(id, record)
	}
	return h.saveDesktopState(id, record)
}

func (h *hostRegistry) requestDesktopStop(id string, record hostRecord) error {
	if record.stop == nil {
		record.stop = &desktopStop{}
	}
	sessions := record.sessions
	record.sessions = nil
	if err := h.saveDesktopState(id, record); err != nil {
		return err
	}
	if sessions != nil {
		if sessions.timer != nil {
			sessions.timer.Stop()
		}
		for _, disconnect := range sessions.connections {
			disconnect()
		}
	}
	h.scheduleDesktopStop(id, record)
	return nil
}

// A second window can share an in-flight start. Each opening has a deadline
// in case its websocket is never established.
func (h *hostRegistry) prepareDesktopLaunch(id string, record hostRecord) (bool, error) {
	shouldStart := !record.desktopReady() && !record.starting &&
		!(record.registration.Desktop.Enabled && record.stop == nil && record.registration.Agent["status"] == "starting")
	if shouldStart {
		record.starting = true
		record.stop = nil
	}
	if record.sessions == nil {
		record.sessions = &desktopSessions{}
	}
	if err := h.saveDesktopState(id, record); err != nil {
		return false, err
	}
	h.stopWhenIdle(id, record.sessions, time.Minute)
	return shouldStart, nil
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
	if stop == nil || stop.running || record.starting || record.packageCommand || record.switchingRuntime() || !record.until.After(time.Now()) || time.Now().Before(stop.retryAt) || record.stopStatus() == "disabled" {
		return
	}
	if record.registration.ControlService == "" {
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
	protocol, _ := attempted.registration.Agent["controller_protocol"].(float64)
	if protocol < 2 && failure == nil && accepted {
		// The standalone agent acknowledges the command before its heartbeat.
		stop.confirmedService = current.registration.ControlService
		stop.retryAt = time.Now().Add(5 * time.Second)
		return
	}
	if failure != nil || !accepted || !hasEnabled || enabled || !hasReady || ready {
		stop.retryAt = time.Now().Add(5 * time.Second)
		return
	}
	// The next heartbeat will schedule a new stop if a controller restores
	// an old start intent. An idle device needs no recurring stop command.
	current.stop = nil
	current.registration.Desktop.Enabled = false
	current.registration.Desktop.Ready = false
	current.registration.Agent = state
	h.hosts[id] = current
	h.persist()
}
