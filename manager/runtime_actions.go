package main

import "net/http"

const removedVersion = "0.0.0-uninstalled"

func (r hostRecord) isRemoved() bool { return r.registration.Agent["version"] == removedVersion }

// Installation and user preference are independent. An unknown device has
// no opt-out, and an absent runtime can never accept a desktop connection.
func (r hostRecord) accessEnabled() bool {
	return r.registration.ControlService != "" && !r.accessDisabled &&
		runtimeInstalled(r.registration.Agent, r.registration.Desktop.Ready)
}

func runtimeInstalled(agent map[string]any, ready bool) bool {
	version, _ := agent["version"].(string)
	protocol, _ := agent["controller_protocol"].(float64)
	return version != removedVersion && (agent["installed"] == true || protocol < 2 && (version != "" || ready))
}

func (r hostRecord) switchingRuntime() bool {
	deployment, _ := r.registration.Agent["deployment"].(map[string]any)
	stage, _ := deployment["stage"].(string)
	return stage != "" && stage != "completed" && stage != "failed" && stage != "idle"
}

// Called with h.mu held. The PC installer always requests /manifest. Select
// the replacement only for this authenticated device; archive URLs themselves
// remain immutable, including while a second device installs the full runtime.
func (h *hostRegistry) changeRuntime(w http.ResponseWriter, r *http.Request, id string, record hostRecord, action string) {
	protocol, _ := record.registration.Agent["controller_protocol"].(float64)
	if protocol < 2 || action == "uninstall" && record.manifestDevice == "" {
		h.mu.Unlock()
		writeFailure(w, 409, "client_update_required")
		return
	}
	if record.starting || record.stop != nil && record.stop.running {
		h.mu.Unlock()
		writeFailure(w, 409, "desktop_control_busy")
		return
	}
	updated := record
	updated.packageCommand = true
	h.hosts[id] = updated
	h.mu.Unlock()
	defer func() {
		h.mu.Lock()
		defer h.mu.Unlock()
		current := h.hosts[id]
		if current.sameDevice(record) {
			current.packageCommand = false
			h.hosts[id] = current
		}
	}()
	removing := action == "uninstall"
	if removing {
		reply, failure := forwardHostControl(r.Context(), record, "stop")
		state, _ := reply["state"].(map[string]any)
		if failure != nil {
			writeFailure(w, failure.status, failure.code)
			return
		}
		if reply["accepted"] != true || state["enabled"] != false || state["ready"] != false {
			writeFailure(w, 409, "control_rejected")
			return
		}
		action = "install"
	}
	// Arm the manifest only after shutdown is confirmed. A failed stop must
	// not change what a future installation downloads.
	h.mu.Lock()
	current := h.hosts[id]
	if !current.sameDevice(record) || current.registration.ControlService != record.registration.ControlService {
		h.mu.Unlock()
		writeFailure(w, 409, "device_unavailable")
		return
	}
	// A re-registered PC may have a different host ID. An earlier removal
	// for the same device must not override its explicit reinstall.
	var superseded []string
	for otherID, other := range h.hosts {
		if otherID != id && other.sameDevice(current) && other.removal {
			superseded = append(superseded, otherID)
			other.removal = false
			h.hosts[otherID] = other
		}
	}
	current.removal = removing
	h.hosts[id] = current
	if err := h.persist(); err != nil {
		for _, otherID := range superseded {
			other := h.hosts[otherID]
			other.removal = true
			h.hosts[otherID] = other
		}
		current.removal = record.removal
		h.hosts[id] = current
		h.mu.Unlock()
		writeFailure(w, 500, "desktop_preference_save_failed")
		return
	}
	h.mu.Unlock()
	reply, failure := forwardHostControl(r.Context(), record, action)
	if failure != nil {
		writeFailure(w, failure.status, failure.code)
		return
	}
	h.mu.Lock()
	current = h.hosts[id]
	if current.sameDevice(record) && current.registration.ControlService == record.registration.ControlService {
		// Reconnecting viewers can reuse the session count across an update.
		// Uninstall has already stopped the runtime and clears that obligation.
		if removing {
			current.sessions = nil
		}
		if state, ok := reply["state"].(map[string]any); ok {
			current.registration.Agent = state
		}
		h.hosts[id] = current
		h.persist()
	}
	h.mu.Unlock()
	writeJSON(w, http.StatusAccepted, reply)
}

func (h *hostRegistry) wantsRemoval(r *http.Request, target string) bool {
	device := currentDeviceID(r)
	if device == "" {
		return false
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	for _, record := range h.hosts {
		if record.owner == owner(r) && record.manifestDevice == device && record.registration.Agent["target"] == target && record.removal {
			return true
		}
	}
	return false
}
