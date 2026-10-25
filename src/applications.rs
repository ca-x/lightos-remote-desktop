//! Application launch isolation for an owned desktop. The private D-Bus and
//! display isolate GUI clients; Chromium also needs a separate profile root
//! because its process singleton is keyed by that root, not by the display.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) struct Applications {
    pub profile_root: PathBuf,
}

fn quote(value: &OsStr) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"))
}

fn desktop_argument(path: &Path) -> String {
    let quoted = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!("\"{}\"", quoted.replace('\\', "\\\\").replace('%', "%%"))
}

fn executable(search: &OsStr, bin: &Path, name: &str) -> Option<PathBuf> {
    std::env::split_paths(search)
        .map(|p| p.join(name))
        .find(|p| {
            p.metadata()
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                && fs::canonicalize(p).is_ok_and(|p| !p.starts_with(bin))
        })
        .and_then(|p| {
            if p.is_absolute() {
                Some(p)
            } else {
                std::env::current_dir().ok().map(|cwd| cwd.join(p))
            }
        })
}

fn write_atomic(path: &Path, source: &str, mode: u32) -> Result<()> {
    if fs::symlink_metadata(path)
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o777 == mode)
        && fs::read(path).ok().as_deref() == Some(source.as_bytes())
    {
        return Ok(());
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("launcher parent")?)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    file.write_all(source.as_bytes())?;
    file.as_file().sync_all()?;
    // Close the writable descriptor before publishing an executable script.
    // Readers may exec it immediately after the rename.
    file.into_temp_path().persist(path)?;
    Ok(())
}

fn directory(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "application directory must be a directory, not a symlink"
    );
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() },
        "application directory has another owner"
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn data_home() -> Result<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .context("HOME or an absolute XDG_DATA_HOME is required for desktop applications")?;
    Ok(data)
}

pub(crate) fn profile_root(runtime: &Path) -> Result<PathBuf> {
    profile_root_in(runtime, &data_home()?)
}

fn profile_root_in(runtime: &Path, data: &Path) -> Result<PathBuf> {
    let runtime = fs::canonicalize(runtime)?;
    let scope = format!(
        "{:x}",
        Sha256::digest(runtime.as_os_str().as_encoded_bytes())
    );
    let profile = data
        .join("lightos-remote-desktop/application-profiles")
        .join(&scope[..24]);
    ensure!(
        profile.is_absolute(),
        "application profile directory must be absolute"
    );
    Ok(profile)
}

pub(crate) fn prepare(runtime: &Path, omarchy: Option<&Path>) -> Result<Applications> {
    let mut paths = Vec::new();
    if let Some(root) = omarchy {
        paths.push(root.join("bin"));
    }
    if let Some(search) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&search));
    }
    prepare_in(runtime, &data_home()?, &std::env::join_paths(paths)?)
}

fn prepare_in(runtime: &Path, data: &Path, search: &OsStr) -> Result<Applications> {
    let runtime = fs::canonicalize(runtime)?;
    let profile_root = profile_root_in(&runtime, data)?;
    directory(&profile_root)?;
    let bin = runtime.join("bin");
    directory(&bin)?;
    let entries = runtime.join("desktop-entries");
    directory(&entries)?;
    let environment = runtime.join("application-environment.sh");
    let source = format!(
        "# Generated for this owned desktop only. Do not import into systemd --user.\n\
         if [ \"${{XDG_RUNTIME_DIR:-}}\" != {runtime} ] || [ \"${{DBUS_SESSION_BUS_ADDRESS:-}}\" != {bus} ]; then\n\
         printf '%s\\n' 'LightOS: application launcher belongs to a different desktop session' >&2; exit 1\nfi\n\
         case ${{WAYLAND_DISPLAY:-}} in /*) lightos_app_socket=$WAYLAND_DISPLAY ;; *) lightos_app_socket=$XDG_RUNTIME_DIR/${{WAYLAND_DISPLAY:-}} ;; esac\n\
         case $lightos_app_socket in {runtime}/wayland-*) ;; *) printf '%s\\n' 'LightOS: application display is outside its desktop session' >&2; exit 1 ;; esac\n\
         if [ ! -S \"$lightos_app_socket\" ]; then printf '%s\\n' 'LightOS: application desktop display is unavailable' >&2; exit 1; fi\n\
         unset lightos_app_socket CHROME_USER_DATA_DIR\nexport CHROME_CONFIG_HOME={profile}\n",
        runtime = quote(runtime.as_os_str()),
        bus = quote(OsStr::new(&format!("unix:path={}/bus", runtime.display()))),
        profile = quote(profile_root.as_os_str()),
    );
    write_atomic(&environment, &source, 0o600)?;
    for (name, source) in [
        ("uwsm-app", include_str!("../scripts/session-uwsm-app.sh")),
        (
            "chromium-launch",
            include_str!("../scripts/session-chromium.sh"),
        ),
        (
            "omarchy-launch-browser",
            include_str!("../scripts/session-browser.sh"),
        ),
    ] {
        write_atomic(&bin.join(name), source, 0o700)?;
    }
    // Resolve real programs before adding our wrappers. Desktop Exec entries
    // may use absolute browser paths, so wrap GTK/URL activation as well.
    for name in [
        "gtk-launch",
        "xdg-open",
        "gio",
        "omarchy-launch-webapp",
        "google-chrome",
        "google-chrome-stable",
        "google-chrome-beta",
        "google-chrome-unstable",
        "chromium",
        "chromium-browser",
        "brave",
        "brave-browser",
        "brave-browser-stable",
        "microsoft-edge",
        "microsoft-edge-stable",
        "vivaldi",
        "vivaldi-stable",
        "opera",
        "helium",
    ] {
        if let Some(program) = executable(search, &bin, name) {
            let mut source = format!(
                "#!/bin/sh\nset -eu\n. {}\nLIGHTOS_REAL_PROGRAM={}\n",
                quote(environment.as_os_str()),
                quote(program.as_os_str())
            );
            match name {
                "gtk-launch" => {
                    let gio = executable(search, &bin, "gio").unwrap_or_default();
                    source.push_str(&format!("LIGHTOS_REAL_GIO={}\nLIGHTOS_DESKTOP_ENTRIES={}\nLIGHTOS_BROWSER_EXEC={}\n", quote(gio.as_os_str()), quote(entries.as_os_str()), quote(OsStr::new(&desktop_argument(&bin.join("chromium-launch"))))));
                    source.push_str(include_str!("../scripts/session-gtk-launch.sh"));
                }
                "xdg-open" | "gio" => {
                    source.push_str(include_str!("../scripts/session-url-launch.sh"))
                }
                "omarchy-launch-webapp" => {
                    source.push_str("exec \"$LIGHTOS_REAL_PROGRAM\" \"$@\"\n")
                }
                _ => source.push_str(&format!(
                    "exec {} \"$LIGHTOS_REAL_PROGRAM\" \"$@\"\n",
                    quote(bin.join("chromium-launch").as_os_str())
                )),
            }
            write_atomic(&bin.join(name), &source, 0o700)?;
        }
    }
    Ok(Applications { profile_root })
}

impl Applications {
    pub fn apply(&self, command: &mut Command) {
        command
            .env("CHROME_CONFIG_HOME", &self.profile_root)
            .env_remove("CHROME_USER_DATA_DIR");
    }

    pub fn update_activation(&self, runtime: &Path) -> Result<()> {
        let output = Command::new("timeout")
            .args(["--kill-after=1", "3", "dbus-update-activation-environment"])
            .arg(format!(
                "CHROME_CONFIG_HOME={}",
                self.profile_root.display()
            ))
            .arg("CHROME_USER_DATA_DIR=")
            .env("XDG_RUNTIME_DIR", runtime)
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/bus", runtime.display()),
            )
            .output()?;
        ensure!(
            output.status.success(),
            "updating private application activation environment: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf, UnixListener) {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("r");
        let host = root.path().join("host-bin");
        fs::create_dir(&runtime).unwrap();
        fs::create_dir(&host).unwrap();
        let data = root.path().join("data with ' and $(literal)");
        let program = host.join("gtk-launch");
        fs::write(&program, "#!/bin/sh\nprintf '%s\\0' \"$CHROME_CONFIG_HOME\" \"${CHROME_USER_DATA_DIR-unset}\" \"$XDG_CONFIG_HOME\" \"$DISPLAY\" \"$DBUS_SESSION_BUS_ADDRESS\" \"$@\" > \"$TEST_OUTPUT\"\n").unwrap();
        fs::set_permissions(program, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = UnixListener::bind(runtime.join("wayland-1")).unwrap();
        (root, runtime, host, data, socket)
    }

    fn command(runtime: &Path, program: &Path, output: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env("XDG_RUNTIME_DIR", runtime)
            .env("WAYLAND_DISPLAY", "wayland-1")
            .env("DISPLAY", ":93")
            .env(
                "DBUS_SESSION_BUS_ADDRESS",
                format!("unix:path={}/bus", runtime.display()),
            )
            .env("CHROME_CONFIG_HOME", "/host/chrome")
            .env("CHROME_USER_DATA_DIR", "/host/profile")
            .env("XDG_CONFIG_HOME", "/host/native-app-config")
            .env("TEST_OUTPUT", output);
        command
    }

    #[test]
    fn launches_use_private_profiles_without_rewriting_arguments_or_native_config() {
        let (root, runtime, host, data, _socket) = fixture();
        let apps = prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        let output = root.path().join("arguments");
        let args = [
            "App with spaces.desktop",
            "https://example.invalid/?q=$(touch BAD);a='b'",
            "--profile-directory=Profile 2",
        ];
        let result = command(&runtime, &runtime.join("bin/gtk-launch"), &output)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes = fs::read(output).unwrap();
        let values: Vec<_> = bytes
            .split(|b| *b == 0)
            .filter(|v| !v.is_empty())
            .map(|v| String::from_utf8(v.to_vec()).unwrap())
            .collect();
        assert_eq!(values[0], apps.profile_root.to_string_lossy());
        assert_eq!(&values[1..4], &["unset", "/host/native-app-config", ":93"]);
        assert_eq!(values[4], format!("unix:path={}/bus", runtime.display()));
        assert_eq!(&values[5..], &args);
        assert!(!root.path().join("BAD").exists());
        assert_eq!(
            fs::metadata(&apps.profile_root)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn wrong_runtime_bus_or_display_fails_without_starting_an_application() {
        let (root, runtime, host, data, _socket) = fixture();
        prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        let output = root.path().join("must-not-exist");
        for (variable, value) in [
            ("XDG_RUNTIME_DIR", "/physical/runtime"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/physical/bus"),
            ("WAYLAND_DISPLAY", "/physical/wayland-1"),
            ("WAYLAND_DISPLAY", "wayland-missing"),
        ] {
            let result = command(&runtime, &runtime.join("bin/gtk-launch"), &output)
                .env(variable, value)
                .output()
                .unwrap();
            assert!(!result.status.success(), "{variable}");
            assert!(!output.exists());
            assert!(String::from_utf8_lossy(&result.stderr).contains("LightOS:"));
        }
    }

    #[test]
    fn profile_identity_survives_refresh_and_different_sessions_have_different_profiles() {
        let (_root, runtime, host, data, _socket) = fixture();
        let first = prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        fs::write(first.profile_root.join("retained-state"), "saved").unwrap();
        let other = runtime.parent().unwrap().join("other");
        fs::create_dir(&other).unwrap();
        let second = prepare_in(&other, &data, host.as_os_str()).unwrap();
        assert_ne!(first.profile_root, second.profile_root);
        let search = std::env::join_paths([runtime.join("bin"), host]).unwrap();
        let refreshed = prepare_in(&runtime, &data, &search).unwrap();
        assert_eq!(first.profile_root, refreshed.profile_root);
        assert_eq!(
            fs::read_to_string(first.profile_root.join("retained-state")).unwrap(),
            "saved"
        );
        let script = fs::read_to_string(runtime.join("bin/gtk-launch")).unwrap();
        assert!(script.contains("host-bin/gtk-launch"));
        assert!(!script.contains("exec '/physical"));
    }

    #[test]
    fn uwsm_bridge_refreshes_old_process_environments() {
        let (root, runtime, host, data, _socket) = fixture();
        let apps = prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        let output = root.path().join("arguments");
        let result = command(&runtime, &runtime.join("bin/uwsm-app"), &output)
            .arg("--")
            .arg(host.join("gtk-launch"))
            .arg("old-shell.desktop")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let bytes = fs::read(output).unwrap();
        assert!(bytes.starts_with(apps.profile_root.to_string_lossy().as_bytes()));
        assert!(bytes.ends_with(b"old-shell.desktop\0"));
    }

    #[test]
    fn browser_commands_default_to_native_wayland_and_keep_explicit_platform_overrides() {
        let (root, runtime, host, data, _socket) = fixture();
        fs::copy(host.join("gtk-launch"), host.join("google-chrome-stable")).unwrap();
        prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        let output = root.path().join("arguments");
        for launcher in ["google-chrome-stable", "uwsm-app"] {
            let mut child = command(&runtime, &runtime.join("bin").join(launcher), &output);
            if launcher == "uwsm-app" {
                child.arg("--").arg(host.join("google-chrome-stable"));
            }
            let result = child
                .arg("https://example.invalid/?x=$(literal)")
                .status()
                .unwrap();
            assert!(result.success());
            let bytes = fs::read(&output).unwrap();
            assert!(
                bytes
                    .split(|b| *b == 0)
                    .any(|v| v == b"https://example.invalid/?x=$(literal)")
            );
            assert!(bytes.ends_with(b"--ozone-platform=wayland\0"));
            assert!(String::from_utf8_lossy(&bytes).contains("--user-data-dir="));
        }
        assert!(
            command(&runtime, &runtime.join("bin/google-chrome-stable"), &output)
                .arg("--ozone-platform=x11")
                .status()
                .unwrap()
                .success()
        );
        let bytes = fs::read(&output).unwrap();
        assert!(
            bytes
                .split(|b| *b == 0)
                .any(|v| v == b"--ozone-platform=x11")
        );
        assert!(!String::from_utf8_lossy(&bytes).contains("--ozone-platform=wayland"));
    }

    #[test]
    fn browser_desktop_entries_keep_metadata_and_exec_fields_in_a_private_copy() {
        let (root, runtime, host, data, _socket) = fixture();
        fs::copy(host.join("gtk-launch"), host.join("gio")).unwrap();
        let entry = data.join("applications/google-chrome.desktop");
        fs::create_dir_all(entry.parent().unwrap()).unwrap();
        let original = "[Desktop Entry]\nType=Application\nName=Custom Chrome\nIcon=google-chrome\nDBusActivatable=true\nExec=/usr/bin/google-chrome-stable --profile-directory=\"Profile 2\" %U\n";
        fs::write(&entry, original).unwrap();
        prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        let output = root.path().join("arguments");
        let result = command(&runtime, &runtime.join("bin/gtk-launch"), &output)
            .env("XDG_DATA_HOME", &data)
            .args([
                "--",
                "google-chrome.desktop",
                "https://example.invalid/?x=a&b=c",
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read_to_string(entry).unwrap(), original);
        let copied =
            fs::read_to_string(runtime.join("desktop-entries/google-chrome.desktop")).unwrap();
        assert!(copied.contains("Name=Custom Chrome\n"));
        assert!(copied.contains("DBusActivatable=false\n"));
        assert!(copied.contains(
            "chromium-launch\" /usr/bin/google-chrome-stable --profile-directory=\"Profile 2\" %U"
        ));
        assert!(
            fs::read(output)
                .unwrap()
                .ends_with(b"https://example.invalid/?x=a&b=c\0")
        );
    }

    #[test]
    fn browser_channels_keep_distinct_profiles_and_sandbox_brokers_are_not_rewritten() {
        let (root, runtime, host, data, _socket) = fixture();
        let names = [
            "google-chrome-stable",
            "google-chrome-beta",
            "brave-browser",
            "brave-browser-beta",
            "microsoft-edge-stable",
            "microsoft-edge-beta",
        ];
        for name in names {
            fs::copy(host.join("gtk-launch"), host.join(name)).unwrap();
        }
        let apps = prepare_in(&runtime, &data, host.as_os_str()).unwrap();
        let output = root.path().join("arguments");
        let mut profiles = std::collections::HashSet::new();
        for name in names {
            assert!(
                command(&runtime, &runtime.join("bin/uwsm-app"), &output)
                    .arg("--")
                    .arg(host.join(name))
                    .status()
                    .unwrap()
                    .success()
            );
            let bytes = fs::read(&output).unwrap();
            let profile = bytes
                .split(|b| *b == 0)
                .find_map(|b| b.strip_prefix(b"--user-data-dir="))
                .unwrap();
            let profile = PathBuf::from(String::from_utf8(profile.to_vec()).unwrap());
            assert!(profile.starts_with(&apps.profile_root));
            assert!(profiles.insert(profile));
        }
        fs::remove_file(&output).unwrap();
        let refused = command(&runtime, &runtime.join("bin/chromium-launch"), &output)
            .env("LIGHTOS_BROWSER_DESKTOP_ID", "google-chrome.desktop")
            .arg("/usr/bin/flatpak")
            .args(["run", "com.google.Chrome"])
            .output()
            .unwrap();
        assert!(!refused.status.success());
        assert!(!output.exists());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("separate adapter"));
    }
}
