package main

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"time"
)

type powerJob struct {
	action, nonce, verifier, controlService string
	claimed                                 bool
	done                                    chan struct{}
	response                                *bool
}

func powerVerifier(record hostRecord) string {
	p, _ := record.registration.Agent["permissions"].(map[string]any)
	v, _ := p["power_verifier"].(string)
	if len(v) != 64 {
		return ""
	}
	if _, err := hex.DecodeString(v); err != nil {
		return ""
	}
	return v
}

// Called with the registry locked. A wake for a cancelled status check must
// never turn into a screen, input or lock-screen authorization prompt.
func (h *hostRegistry) allowPermissionPrompt(id string) {
	if h.permissionPrompts == nil {
		h.permissionPrompts = make(map[string]time.Time)
	}
	h.permissionPrompts[id] = time.Now().Add(18 * time.Second)
}

func (h *hostRegistry) powerControl(w http.ResponseWriter, r *http.Request, id string, record hostRecord, action string) {
	// Takes ownership of the caller's registry lock.
	verifier := powerVerifier(record)
	if verifier == "" || !runtimeInstalled(record.registration.Agent, record.registration.Desktop.Ready) {
		h.mu.Unlock()
		writeFailure(w, 409, "power_update_required")
		return
	}
	p, _ := record.registration.Agent["permissions"].(map[string]any)
	if h.powerJobs[id] != nil || time.Now().Before(h.permissionPrompts[id]) || p["requesting"] == true {
		h.mu.Unlock()
		writeFailure(w, 409, "power_busy")
		return
	}
	var nonce [32]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		h.mu.Unlock()
		writeFailure(w, 500, "power_unavailable")
		return
	}
	job := &powerJob{action: action, nonce: hex.EncodeToString(nonce[:]), verifier: verifier, controlService: record.registration.ControlService, done: make(chan struct{})}
	if h.powerJobs == nil {
		h.powerJobs = make(map[string]*powerJob)
	}
	h.powerJobs[id] = job
	h.mu.Unlock()
	defer func() { h.mu.Lock(); delete(h.powerJobs, id); h.mu.Unlock() }()
	// The PC may already have a read-only permissions probe in flight, in
	// which case it acknowledges a wake without starting another process.
	timeout := 105 * time.Second
	if action == "read" {
		timeout = 15 * time.Second
	}
	parent := r.Context()
	if action != "read" {
		// The user may close the page while the host's system dialog is open.
		// Serialize that explicit change through device confirmation or timeout.
		parent = context.WithoutCancel(parent)
	}
	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()
	deliveryDeadline := time.Now().Add(10 * time.Second)
	wakeAccepted := false
	for {
		h.mu.Lock()
		claimed := job.claimed
		h.mu.Unlock()
		if !claimed {
			if time.Now().After(deliveryDeadline) {
				writeFailure(w, 503, "power_unavailable")
				return
			}
			if !wakeAccepted {
				reply, failure := forwardHostControl(ctx, record, "permissions")
				if failure == nil {
					state, _ := reply["state"].(map[string]any)
					permissions, _ := state["permissions"].(map[string]any)
					wakeAccepted = record.registration.Platform == "darwin" || permissions["requesting"] == true
				} else {
					h.mu.Lock()
					claimed = job.claimed
					h.mu.Unlock()
					if !claimed {
						writeFailure(w, failure.status, "power_unavailable")
						return
					}
				}
			}
		}
		var retry <-chan time.Time
		if !claimed {
			retry = time.After(1100 * time.Millisecond)
		}
		select {
		case <-job.done:
			if job.response == nil || action != "read" && *job.response != (action == "enable") {
				writeFailure(w, 409, "power_not_applied")
				return
			}
			writeJSON(w, 200, map[string]any{"enabled": *job.response})
			return
		case <-ctx.Done():
			writeFailure(w, 504, "power_unavailable")
			return
		case <-retry:
		}
	}
}

// Worker calls use the logged-in account, the registered device identity and
// proof of the private credential. Only its hash is carried by the PC report.
func (h *hostRegistry) powerWorker(w http.ResponseWriter, r *http.Request) {
	id := hostID(owner(r), r.PathValue("registration"))
	key := r.Header.Get("X-Lazycat-Power-Key")
	sum := sha256.Sum256([]byte(key))
	h.mu.Lock()
	defer h.mu.Unlock()
	record, ok := h.hosts[id]
	verifier := powerVerifier(record)
	if !ok || record.owner != owner(r) || !record.until.After(time.Now()) || !record.isCurrentDevice(currentDeviceID(r)) || len(key) != 64 ||
		(verifier != "" && subtle.ConstantTimeCompare([]byte(verifier), []byte(hex.EncodeToString(sum[:]))) != 1) {
		writeFailure(w, 403, "device_unavailable")
		return
	}
	job := h.powerJobs[id]
	valid := verifier != "" && job != nil && job.verifier == verifier && job.controlService == record.registration.ControlService && !record.removal && !record.packageCommand && !record.switchingRuntime()
	switch r.PathValue("phase") {
	case "claim":
		if valid && !job.claimed {
			job.claimed = true
			writeJSON(w, 200, map[string]string{"action": job.action, "nonce": job.nonce})
			return
		}
		action := "idle"
		if time.Now().Before(h.permissionPrompts[id]) {
			delete(h.permissionPrompts, id)
			action = "permissions"
		}
		writeJSON(w, 200, map[string]string{"action": action})
	case "result":
		var body struct {
			Nonce   string `json:"nonce"`
			Enabled *bool  `json:"enabled"`
		}
		if !valid || !job.claimed || json.NewDecoder(io.LimitReader(r.Body, 1024)).Decode(&body) != nil || body.Nonce != job.nonce {
			writeFailure(w, 409, "expired_power_request")
			return
		}
		select {
		case <-job.done:
		default:
			job.response = body.Enabled
			close(job.done)
		}
		w.WriteHeader(http.StatusNoContent)
	default:
		writeFailure(w, 400, "invalid_action")
	}
}
