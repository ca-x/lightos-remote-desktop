//! Startup dependencies are distinct from optional desktop features.
use crate::config::Config;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    OwnedVirtual,
    PersistentAttach,
    ExternalWayland,
    ExternalX11,
}

impl SessionMode {
    pub fn from_config(config: &Config) -> Self {
        if config.x11_display.is_some() {
            Self::ExternalX11
        } else if config.wayland_socket.is_some() {
            Self::ExternalWayland
        } else if config.session_runtime.is_some() {
            Self::PersistentAttach
        } else {
            Self::OwnedVirtual
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Dependency {
    pub program: PathBuf,
    pub feature: &'static str,
    pub available: bool,
}

#[derive(Debug, Serialize)]
pub struct DependencyReport {
    pub session_mode: SessionMode,
    pub required: Vec<Dependency>,
    pub optional: Vec<Dependency>,
}

pub fn dependencies(
    config: &Config,
    omarchy: bool,
    resolve: impl Fn(&Path) -> Option<PathBuf>,
) -> DependencyReport {
    let session_mode = SessionMode::from_config(config);
    let mut required: Vec<(PathBuf, &'static str, bool)> = vec![];
    let mut optional: Vec<(PathBuf, &'static str, bool)> = vec![];
    if session_mode == SessionMode::OwnedVirtual {
        required.extend([
            (
                config.cage.clone(),
                "desktop startup",
                resolve(&config.cage).is_some(),
            ),
            (
                "Hyprland".into(),
                "desktop startup",
                resolve(Path::new("Hyprland")).is_some(),
            ),
            (
                "dbus-daemon".into(),
                "desktop startup",
                resolve(Path::new("dbus-daemon")).is_some(),
            ),
            (
                "dbus-update-activation-environment".into(),
                "application environment",
                resolve(Path::new("dbus-update-activation-environment")).is_some(),
            ),
        ]);
        if omarchy {
            required.push((
                "quickshell".into(),
                "Omarchy shell",
                resolve(Path::new("quickshell")).is_some(),
            ));
            required.push((
                "timeout".into(),
                "Omarchy configuration check",
                resolve(Path::new("timeout")).is_some(),
            ));
            optional.extend(["gtk-launch", "xdg-settings", "gio"].map(|name| {
                (
                    name.into(),
                    "Omarchy browser launcher",
                    resolve(Path::new(name)).is_some(),
                )
            }));
        } else if config.desktop_config.is_none() {
            let candidates = ["xdg-terminal-exec", "foot", "ghostty", "kitty", "alacritty"];
            required.push((
                "terminal launcher".into(),
                "terminal launcher",
                candidates
                    .iter()
                    .any(|name| resolve(Path::new(name)).is_some()),
            ));
        }
        if config.audio {
            required.extend(
                ["pipewire", "pipewire-pulse", "wireplumber", "pactl"].map(|name| {
                    (
                        name.into(),
                        "isolated audio startup",
                        resolve(Path::new(name)).is_some(),
                    )
                }),
            );
            if !omarchy {
                required.push((
                    "timeout".into(),
                    "isolated audio startup",
                    resolve(Path::new("timeout")).is_some(),
                ));
            }
        }
    } else if session_mode == SessionMode::ExternalX11 {
        required.push((
            "xauth".into(),
            "X11 authentication",
            resolve(Path::new("xauth")).is_some(),
        ));
    }
    if session_mode != SessionMode::ExternalX11 {
        optional.extend(
            ["wl-copy", "wl-paste"]
                .map(|name| (name.into(), "clipboard", resolve(Path::new(name)).is_some())),
        );
    }
    DependencyReport {
        session_mode,
        required: required
            .into_iter()
            .map(|(program, feature, available)| Dependency {
                program,
                feature,
                available,
            })
            .collect(),
        optional: optional
            .into_iter()
            .map(|(program, feature, available)| Dependency {
                program,
                feature,
                available,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn attached_sessions_need_no_owned_startup_tools_and_report_optional_clipboard() {
        for args in [
            vec!["test", "--wayland-socket", "/private/wayland-2"],
            vec!["test", "--session-runtime", "/private/owner"],
        ] {
            let config = Config::try_parse_from(args).unwrap();
            let report = dependencies(&config, true, |_| None);
            assert!(report.required.is_empty());
            assert_eq!(report.optional.len(), 2);
            assert!(report.optional.iter().all(|d| !d.available));
            assert_eq!(report.optional[0].program, Path::new("wl-copy"));
            assert_eq!(report.optional[1].program, Path::new("wl-paste"));
        }
    }

    #[test]
    fn x11_attachment_checks_its_authentication_helper_only() {
        let config = Config::try_parse_from(["test", "--x11-display", ":0"]).unwrap();
        let report = dependencies(&config, true, |_| None);
        assert_eq!(report.session_mode, SessionMode::ExternalX11);
        assert_eq!(report.required.len(), 1);
        assert_eq!(report.required[0].program, Path::new("xauth"));
        assert!(report.optional.is_empty());
    }

    #[test]
    fn niri_startup_requires_native_tools_without_hyprland_or_omarchy() {
        let config = Config::try_parse_from([
            "test",
            "--compositor",
            "niri",
            "--desktop-config",
            "/private/native/config.kdl",
            "--audio",
            "false",
        ])
        .unwrap();
        let report = dependencies(&config, true, |_| None);
        let names: Vec<_> = report
            .required
            .iter()
            .map(|dependency| dependency.program.to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "cage",
                "niri",
                "dbus-daemon",
                "dbus-update-activation-environment"
            ]
        );
    }

    #[test]
    fn owned_startup_requires_its_tools_and_only_enabled_audio_tools() {
        let mut config = Config::try_parse_from(["test", "--audio", "false"]).unwrap();
        let report = dependencies(&config, false, |_| None);
        let names: Vec<_> = report
            .required
            .iter()
            .map(|d| d.program.to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "cage",
                "Hyprland",
                "dbus-daemon",
                "dbus-update-activation-environment",
                "terminal launcher"
            ]
        );
        assert!(report.required.iter().all(|d| !d.available));
        config.audio = true;
        let report = dependencies(&config, true, |_| None);
        for name in [
            "pipewire",
            "pipewire-pulse",
            "wireplumber",
            "pactl",
            "timeout",
            "quickshell",
        ] {
            assert!(
                report.required.iter().any(|d| d.program == Path::new(name)),
                "{name}"
            );
        }
        assert!(
            !report
                .required
                .iter()
                .any(|d| d.program == Path::new("gtk-launch"))
        );
        assert!(
            report
                .optional
                .iter()
                .any(|d| d.program == Path::new("gtk-launch"))
        );
        assert!(
            !report
                .required
                .iter()
                .any(|d| d.program == Path::new("hyprctl"))
        );
    }

    #[test]
    fn isolated_executable_fixture_checks_terminal_alternatives_and_omarchy_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let resolve = |program: &Path| {
            let path = if program.is_absolute() {
                program.to_path_buf()
            } else {
                directory.path().join(program)
            };
            platform_detect::find_executable(&path)
        };
        let mut config = Config::try_parse_from(["test", "--audio", "false"]).unwrap();
        let terminal = directory.path().join("foot");
        fs::write(&terminal, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&terminal, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            !dependencies(&config, false, resolve)
                .required
                .iter()
                .any(|d| d.feature == "terminal launcher" && d.available)
        );
        fs::set_permissions(&terminal, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            dependencies(&config, false, resolve)
                .required
                .iter()
                .any(|d| d.feature == "terminal launcher" && d.available)
        );
        config.desktop_config = Some("/private/config".into());
        assert!(
            !dependencies(&config, false, resolve)
                .required
                .iter()
                .any(|d| d.feature == "terminal launcher")
        );
        config.desktop_config = None;
        let report = dependencies(&config, true, resolve);
        assert!(
            report
                .required
                .iter()
                .any(|d| d.program == Path::new("timeout"))
        );
        assert!(
            report
                .optional
                .iter()
                .any(|d| d.program == Path::new("xdg-settings"))
        );
    }
}
