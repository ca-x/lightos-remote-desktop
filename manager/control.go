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

func (h *hostRegistry) control(w http.ResponseWriter, r *http.Request) {
	action := r.PathValue("action")
	if r.Method != http.MethodPost || (action != "start" && action != "stop" && action != "permissions") {
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
	h.mu.Lock()
	record, ok := h.hosts[r.PathValue("id")]
	h.mu.Unlock()
	if !ok || record.owner != owner(r) || record.registration.ControlService == "" || !record.until.After(time.Now()) {
		writeFailure(w, http.StatusConflict, "device_unavailable")
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 12*time.Second)
	defer cancel()
	api, device, err := ownedDevice(ctx, owner(r), record.registration.DeviceID)
	if err != nil {
		writeFailure(w, 503, "device_unavailable")
		return
	}
	defer api.Close()
	proxy, err := api.NewDeviceProxy(device.GetDeviceApiUrl())
	if err != nil {
		writeFailure(w, 503, "device_unavailable")
		return
	}
	token, err := proxy.GetAuthToken(ctx)
	proxy.Close()
	if err != nil {
		writeFailure(w, 503, "device_authorization_failed")
		return
	}
	target, err := url.Parse(device.GetDeviceApiUrl())
	if err != nil || target.Scheme != "https" || target.Host == "" || target.User != nil || target.RawQuery != "" || target.Fragment != "" {
		writeFailure(w, 503, "device_unavailable")
		return
	}
	target.Path = strings.TrimRight(target.Path, "/") + "/s/" + record.registration.ControlService + "/" + action
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, target.String(), strings.NewReader("{}"))
	if err != nil {
		writeFailure(w, 503, "device_unavailable")
		return
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("lzc_dapi_auth_token", token.Token)
	req.Header.Set("X-Lazycat-Remote-Agent", record.registration.ControlPassword)
	client := &http.Client{Timeout: 12 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	response, err := client.Do(req)
	if err != nil {
		writeFailure(w, 502, "control_unavailable")
		return
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusAccepted && response.StatusCode != http.StatusOK {
		writeFailure(w, 502, "control_rejected")
		return
	}
	var reply any
	if json.NewDecoder(io.LimitReader(response.Body, 8192)).Decode(&reply) != nil {
		writeFailure(w, 502, "invalid_control_response")
		return
	}
	writeJSON(w, http.StatusAccepted, reply)
}
