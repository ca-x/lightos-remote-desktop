package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

type runtimeArtifact struct {
	Target          string `json:"target"`
	Version         string `json:"version"`
	Binary          string `json:"binary"`
	SHA256          string `json:"sha256"`
	ArchiveSHA256   string `json:"archive_sha256"`
	ArchiveSize     int64  `json:"archive_size"`
	DownloadPath    string `json:"download_path"`
	Protocol        int    `json:"runtime_protocol"`
	VirtualFallback bool   `json:"virtual_fallback,omitempty"`
	SigningTeam     string `json:"signing_team,omitempty"`
}
type runtimeCatalog struct {
	Version   string                     `json:"version"`
	Targets   map[string]runtimeArtifact `json:"targets"`
	Removed   map[string]runtimeArtifact `json:"removed"`
	directory string
}

func loadRuntimeCatalog() (*runtimeCatalog, error) {
	directory := os.Getenv("REMOTE_DESKTOP_RUNTIME_DIR")
	if directory == "" {
		directory = "/lzcapp/pkg/content/runtimes"
	}
	data, err := os.ReadFile(filepath.Join(directory, "index.json"))
	if err != nil {
		return nil, err
	}
	if len(data) > 1<<20 {
		return nil, fmt.Errorf("runtime catalog is too large")
	}
	catalog := &runtimeCatalog{directory: directory}
	if err = json.Unmarshal(data, catalog); err != nil {
		return nil, err
	}
	for _, target := range []string{"darwin-arm64", "darwin-x64", "win32-x64", "linux-x64"} {
		full := catalog.Targets[target]
		removed := catalog.Removed[target]
		if err := verifyArtifact(directory, target+".tar.gz", target, catalog.Version, full); err != nil {
			return nil, err
		}
		if err := verifyArtifact(directory, "removed-"+target+".tar.gz", target, removedVersion, removed); err != nil {
			return nil, err
		}
		if full.SigningTeam != removed.SigningTeam || full.DownloadPath == removed.DownloadPath || full.Version == removedVersion {
			return nil, fmt.Errorf("incompatible uninstall package: %s", target)
		}
	}
	return catalog, nil
}

func verifyArtifact(directory, filename, target, version string, artifact runtimeArtifact) error {
	digest, err := hex.DecodeString(artifact.SHA256)
	if artifact.VirtualFallback && target != "linux-x64" || artifact.Target != target || artifact.Version != version || version == "" || artifact.Protocol != 2 || err != nil || len(digest) != 32 || artifact.ArchiveSize < 1 || !strings.HasPrefix(artifact.DownloadPath, "/api/runtimes/"+target+"/") {
		return fmt.Errorf("invalid runtime catalog entry: %s", filename)
	}
	file, err := os.Open(filepath.Join(directory, filename))
	if err != nil {
		return err
	}
	defer file.Close()
	hash := sha256.New()
	size, err := io.Copy(hash, file)
	if err != nil || size != artifact.ArchiveSize || hex.EncodeToString(hash.Sum(nil)) != artifact.ArchiveSHA256 {
		return fmt.Errorf("runtime archive does not match its manifest: %s", filename)
	}
	return nil
}

func (s *server) serveRuntime(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet && r.Method != http.MethodHead {
		w.WriteHeader(405)
		return
	}
	target := r.PathValue("target")
	c := s.runtimes
	artifact, ok := c.Targets[target]
	if !ok {
		http.NotFound(w, r)
		return
	}
	file := r.PathValue("file")
	filename := target + ".tar.gz"
	removed := c.Removed[target]
	if file == "manifest" && s.hosts.wantsRemoval(r, target) || r.URL.Path == removed.DownloadPath {
		artifact = removed
		filename = "removed-" + filename
	}
	if file == "manifest" {
		writeJSON(w, 200, artifact)
		return
	}
	if "/api/runtimes/"+target+"/"+file != artifact.DownloadPath {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "application/gzip")
	w.Header().Set("Content-Disposition", `attachment; filename="`+target+`.tar.gz"`)
	http.ServeFile(w, r, filepath.Join(c.directory, filename))
}

func (c *runtimeCatalog) decorate(target *desktopTarget) {
	if target.Kind != "device" {
		return
	}
	agent := target.agent
	target.RuntimeVersion, _ = agent["version"].(string)
	target.Installed = runtimeInstalled(agent, target.Ready)
	protocol, _ := agent["controller_protocol"].(float64)
	target.ControllerProtocol = int(protocol)
	target.PermissionCheck, _ = agent["permission_check"].(bool)
	target.Deployment, _ = agent["deployment"].(map[string]any)
	target.RuntimeError, _ = agent["error"].(string)
	if target.RuntimeVersion == removedVersion {
		target.RuntimeVersion = ""
		target.Status = "not_installed"
		target.Ready = false
	}
	platform, _ := agent["target"].(string)
	artifact, supported := c.Targets[platform]
	target.RuntimeSupported = supported
	target.CanInstall = target.Managed && protocol >= 2 && supported
	target.AvailableVersion = c.Version
	if supported {
		target.AvailableVersion = artifact.Version
		installedHash, _ := agent["binary_sha256"].(string)
		target.UpdateAvailable = target.Installed && (target.RuntimeVersion != artifact.Version || installedHash != "" && installedHash != artifact.SHA256)
	} else {
		target.UpdateAvailable = target.Installed && target.RuntimeVersion != c.Version
	}
}
