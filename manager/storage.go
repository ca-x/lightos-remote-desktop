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
	ID             string         `json:"id"`
	Owner          string         `json:"owner"`
	Name           string         `json:"name"`
	Platform       string         `json:"platform"`
	DeviceID       string         `json:"device_id,omitempty"`
	BoxID          string         `json:"box_id,omitempty"`
	Managed        *bool          `json:"managed,omitempty"`
	Agent          map[string]any `json:"agent"`
	ManifestDevice string         `json:"manifest_device,omitempty"`
	Removal        bool           `json:"removal,omitempty"`
	AccessDisabled *bool          `json:"access_disabled,omitempty"`
	AccessEnabled  *bool          `json:"access_enabled,omitempty"`
	Stop           bool           `json:"stop_requested,omitempty"`
	ReadyEpoch     string         `json:"ready_epoch,omitempty"`
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
		registration := hostRegistration{Name: host.Name, Platform: host.Platform, DeviceID: host.DeviceID, BoxID: host.BoxID, ControlService: "offline", Agent: host.Agent}
		if host.Managed != nil && !*host.Managed {
			registration.ControlService = ""
		}
		record := hostRecord{owner: host.Owner, registration: registration, manifestDevice: host.ManifestDevice, removal: host.Removal, readyEpoch: host.ReadyEpoch}
		if host.AccessDisabled != nil {
			record.accessDisabled = *host.AccessDisabled
		} else if runtimeInstalled(host.Agent, false) {
			// Older managers also saved "off" for a missing runtime. Only
			// preserve a legacy gate or stop request on an installed device.
			if host.AccessEnabled != nil {
				record.accessDisabled = !*host.AccessEnabled
			} else {
				record.accessDisabled = host.Stop
			}
		}
		if host.Stop && host.BoxID != "" && host.DeviceID != "" {
			record.stop = &desktopStop{}
		}
		h.hosts[host.ID] = record
	}
}

// Called with the registry locked. Credentials never enter persistent metadata.
func (h *hostRegistry) persist() error {
	hosts := make([]savedHost, 0, len(h.hosts))
	for id, record := range h.hosts {
		r := record.registration
		if record.source != nil {
			continue
		}
		managed := r.ControlService != ""
		hosts = append(hosts, savedHost{ID: id, Owner: record.owner, Name: r.Name, Platform: r.Platform, DeviceID: r.DeviceID, BoxID: r.BoxID, Managed: &managed, Agent: r.Agent, ManifestDevice: record.manifestDevice, Removal: record.removal, AccessDisabled: &record.accessDisabled, Stop: record.stop != nil || record.sessions != nil, ReadyEpoch: record.readyEpoch})
	}
	// Stable ordering also avoids needless disk writes on heartbeats.
	sortSavedHosts(hosts)
	data, err := json.Marshal(hosts)
	if err != nil {
		return err
	}
	if bytes.Equal(data, h.persisted) {
		return nil
	}
	if err := os.MkdirAll(storageDirectory(), 0700); err != nil {
		return err
	}
	path := filepath.Join(storageDirectory(), "hosts.json")
	if err := os.WriteFile(path+".tmp", data, 0600); err != nil {
		return err
	}
	if err := os.Rename(path+".tmp", path); err != nil {
		return err
	}
	h.persisted = data
	return nil
}
