package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
)

func storageDirectory() string {
	if directory := os.Getenv("REMOTE_DESKTOP_STATE_DIR"); directory != "" {
		return directory
	}
	return "/lzcapp/var/remote-desktop"
}

type savedHost struct {
	ID       string         `json:"id"`
	Owner    string         `json:"owner"`
	Name     string         `json:"name"`
	Platform string         `json:"platform"`
	DeviceID string         `json:"device_id,omitempty"`
	Agent    map[string]any `json:"agent"`
}

func (h *hostRegistry) restore() {
	data, err := os.ReadFile(filepath.Join(storageDirectory(), "hosts.json"))
	if err != nil {
		return
	}
	var hosts []savedHost
	if json.Unmarshal(data, &hosts) != nil {
		return
	}
	for _, host := range hosts {
		if !validHostID(host.ID) || host.Owner == "" {
			continue
		}
		// Offline placeholders contain no route, device token or runtime password.
		registration := hostRegistration{Name: host.Name, Platform: host.Platform, DeviceID: host.DeviceID, ControlService: "offline", Agent: host.Agent}
		h.hosts[host.ID] = hostRecord{owner: host.Owner, registration: registration}
	}
}

// Called with the registry locked. Credentials never enter persistent metadata.
func (h *hostRegistry) persist() {
	hosts := make([]savedHost, 0, len(h.hosts))
	for id, record := range h.hosts {
		r := record.registration
		if r.ControlService == "" {
			continue
		}
		hosts = append(hosts, savedHost{ID: id, Owner: record.owner, Name: r.Name, Platform: r.Platform, DeviceID: r.DeviceID, Agent: r.Agent})
	}
	// Stable ordering also avoids needless disk writes on heartbeats.
	sortSavedHosts(hosts)
	data, err := json.Marshal(hosts)
	if err != nil || bytes.Equal(data, h.persisted) {
		return
	}
	if os.MkdirAll(storageDirectory(), 0700) != nil {
		return
	}
	path := filepath.Join(storageDirectory(), "hosts.json")
	if os.WriteFile(path+".tmp", data, 0600) == nil && os.Rename(path+".tmp", path) == nil {
		h.persisted = data
	}
}
