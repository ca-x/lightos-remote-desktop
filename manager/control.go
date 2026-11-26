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
	if r.Method != http.MethodPost || (action != "start" && action != "stop" && action != "permissions" && action != "permissions-check" && action != "install" && action != "update" && action != "update-confirmed") {
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
	protocol, _ := record.registration.Agent["controller_protocol"].(float64)
	if protocol >= 2 && action == "stop" {
		err := h.requestDesktopStop(id, record)
		h.mu.Unlock()
		if err != nil {
			writeFailure(w, http.StatusInternalServerError, "desktop_preference_save_failed")
			return
		}
		writeJSON(w, http.StatusAccepted, map[string]bool{"accepted": true, "stop_queued": true})
		return
	}
	if action == "start" && (protocol >= 2 || record.stop != nil) {
		failure := h.beginDesktopStart(id, record)
		if failure != nil {
			h.mu.Unlock()
			writeFailure(w, failure.status, failure.code)
			return
		}
		defer h.finishDesktopStart(id, record)
	}
	h.mu.Unlock()
	if action == "install" || action == "update" || action == "update-confirmed" {
		if protocol < 2 {
			writeFailure(w, 409, "client_update_required")
			return
		}
	}
	reply, failure := forwardHostControl(r.Context(), record, action)
	if failure != nil {
		writeFailure(w, failure.status, failure.code)
		return
	}
	if state, ok := reply["state"].(map[string]any); ok {
		h.mu.Lock()
		if current, exists := h.hosts[id]; exists && current.owner == record.owner && current.registration.ControlService == record.registration.ControlService {
			current.registration.Agent = state
			h.hosts[id] = current
		}
		h.mu.Unlock()
	}
	writeJSON(w, http.StatusAccepted, reply)
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
