package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

type controlFailure struct {
	status int
	code   string
}

func (h *hostRegistry) control(w http.ResponseWriter, r *http.Request) {
	action := r.PathValue("action")
	switch action {
	case "uninstall", "connect", "start", "stop", "permissions", "permissions-check", "install", "update", "update-confirmed":
	default:
		writeFailure(w, http.StatusMethodNotAllowed, "invalid_action")
		return
	}
	if r.Method != http.MethodPost {
		writeFailure(w, http.StatusMethodNotAllowed, "invalid_action")
		return
	}
	if r.Header.Get("Sec-Fetch-Site") == "cross-site" || !strings.HasPrefix(r.Header.Get("Content-Type"), "application/json") {
		writeFailure(w, http.StatusForbidden, "invalid_control_origin")
		return
	}
	if origin := r.Header.Get("Origin"); origin != "" && origin != "https://"+r.Host && origin != "http://"+r.Host {
		writeFailure(w, http.StatusForbidden, "invalid_control_origin")
		return
	}
	id := r.PathValue("id")
	h.mu.Lock()
	record, ok := h.hosts[id]
	if !ok || record.owner != owner(r) || record.registration.ControlService == "" || !record.until.After(time.Now()) {
		h.mu.Unlock()
		writeFailure(w, http.StatusConflict, "device_unavailable")
		return
	}
	if record.packageCommand || record.switchingRuntime() {
		h.mu.Unlock()
		writeFailure(w, 409, "desktop_control_busy")
		return
	}
	if action == "install" || action == "update" || action == "update-confirmed" || action == "uninstall" {
		h.changeRuntime(w, r, id, record, action)
		return
	}
	if action == "start" || action == "stop" {
		if !runtimeInstalled(record.registration.Agent, record.registration.Desktop.Ready) {
			h.mu.Unlock()
			writeFailure(w, http.StatusConflict, "desktop_unavailable")
			return
		}
		enable := action == "start"
		err := h.setDesktopAccess(id, record, enable)
		h.mu.Unlock()
		if err != nil {
			writeFailure(w, http.StatusInternalServerError, "desktop_preference_save_failed")
			return
		}
		writeJSON(w, http.StatusOK, map[string]bool{"accepted": true, "access_enabled": enable})
		return
	}
	if action == "connect" {
		if record.isCurrentDevice(currentDeviceID(r)) || !record.canLaunch() {
			h.mu.Unlock()
			writeFailure(w, http.StatusConflict, "desktop_unavailable")
			return
		}
		start, err := h.prepareDesktopLaunch(id, record)
		if err != nil {
			h.mu.Unlock()
			writeFailure(w, 500, "desktop_preference_save_failed")
			return
		}
		if !start {
			h.mu.Unlock()
			writeJSON(w, http.StatusAccepted, map[string]bool{"accepted": true})
			return
		}
		defer h.finishDesktopStart(id, record)
		action = "start"
	}
	h.mu.Unlock()
	ctx, cancel := context.WithTimeout(r.Context(), 12*time.Second)
	defer cancel()
	reply, failure := forwardHostControl(ctx, record, action)
	if failure != nil {
		writeFailure(w, failure.status, failure.code)
		return
	}
	state, _ := reply["state"].(map[string]any)
	if action == "permissions" && needsWindowsLockAuthorization(state) {
		retryWindowsLockAuthorization(ctx, record, reply)
		state, _ = reply["state"].(map[string]any)
	}
	if state != nil {
		h.mu.Lock()
		if current, exists := h.hosts[id]; exists && current.owner == record.owner && current.registration.ControlService == record.registration.ControlService {
			current.registration.Agent = state
			h.hosts[id] = current
		}
		h.mu.Unlock()
	}
	writeJSON(w, http.StatusAccepted, reply)
}

func needsWindowsLockAuthorization(state map[string]any) bool {
	permissions, _ := state["permissions"].(map[string]any)
	return state["target"] == "win32-x64" &&
		permissions["can_start"] == true && permissions["lock_screen_authorized"] == false && permissions["requesting"] != true
}

// A read-only controller probe can briefly occupy the permission worker.
// Retry an explicitly requested prompt once when it did not start.
func retryWindowsLockAuthorization(ctx context.Context, record hostRecord, reply map[string]any) {
	timer := time.NewTimer(350 * time.Millisecond)
	defer timer.Stop()
	select {
	case <-timer.C:
		authorization, failure := forwardHostControl(ctx, record, "permissions")
		if state, ok := authorization["state"].(map[string]any); failure == nil && ok {
			reply["state"] = state
			if !needsWindowsLockAuthorization(state) {
				return
			}
		}
	case <-ctx.Done():
	}
	reply["permission_request_deferred"] = true
}

func forwardHostControl(parent context.Context, record hostRecord, action string) (map[string]any, *controlFailure) {
	ctx, cancel := context.WithTimeout(parent, 12*time.Second)
	defer cancel()
	api, device, err := ownedDevice(ctx, record.owner, record.registration.DeviceID)
	if err != nil {
		return nil, &controlFailure{503, "device_unavailable"}
	}
	defer api.Close()
	proxy, err := api.NewDeviceProxy(device.GetDeviceApiUrl())
	if err != nil {
		return nil, &controlFailure{503, "device_unavailable"}
	}
	token, err := proxy.GetAuthToken(ctx)
	proxy.Close()
	if err != nil {
		return nil, &controlFailure{503, "device_authorization_failed"}
	}
	target, err := url.Parse(device.GetDeviceApiUrl())
	if err != nil || target.Scheme != "https" || target.Host == "" || target.User != nil || target.RawQuery != "" || target.Fragment != "" {
		return nil, &controlFailure{503, "device_unavailable"}
	}
	target.Path = strings.TrimRight(target.Path, "/") + "/s/" + record.registration.ControlService + "/" + action
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, target.String(), strings.NewReader("{}"))
	if err != nil {
		return nil, &controlFailure{503, "device_unavailable"}
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("lzc_dapi_auth_token", token.Token)
	req.Header.Set("X-Lazycat-Remote-Agent", record.registration.ControlPassword)
	client := &http.Client{Timeout: 12 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	response, err := client.Do(req)
	if err != nil {
		return nil, &controlFailure{502, "control_unavailable"}
	}
	defer response.Body.Close()
	if response.StatusCode == http.StatusConflict {
		var failure struct {
			Error string `json:"error"`
		}
		if json.NewDecoder(io.LimitReader(response.Body, 8192)).Decode(&failure) == nil && failure.Error == "update_confirmation_required" {
			return nil, &controlFailure{409, failure.Error}
		}
	}
	if response.StatusCode != http.StatusAccepted && response.StatusCode != http.StatusOK {
		return nil, &controlFailure{502, "control_rejected"}
	}
	var reply map[string]any
	if json.NewDecoder(io.LimitReader(response.Body, 8192)).Decode(&reply) != nil {
		return nil, &controlFailure{502, "invalid_control_response"}
	}
	return reply, nil
}
