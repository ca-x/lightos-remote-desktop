#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn config(arguments: &[&str]) -> Config {
        Config::try_parse_from(arguments).unwrap()
    }

    #[test]
    fn rejects_missing_or_default_password() {
        for password in ["", "changeme", "password", "lightos"] {
            assert!(
                config(&["test", "--password", password])
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn accepts_explicit_runtime_configuration() {
        let settings = config(&["test", "--password", "some-long-secret", "--port", "18766"]);
        assert!(settings.validate().is_ok());
        assert_eq!(settings.port, 18766);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_runtime_paths_that_exceed_hyprland_socket_budget() {
        let mut settings = config(&["test", "--password", "some-long-secret"]);
        settings.runtime_dir = Some(crate::session::DEFAULT_SESSION_RUNTIME.into());
        assert!(settings.validate().is_ok());
        settings.runtime_dir = Some(crate::session::PREVIOUS_SESSION_RUNTIME.into());
        let error = settings.validate().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("too long for Hyprland IPC sockets")
        );
    }

    #[test]
    fn rejects_unbounded_media_sizes_and_rates() {
        for argument in ["--width", "--height", "--fps", "--quality"] {
            assert!(
                config(&["test", "--password", "some-long-secret", argument, "0"])
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn rejects_credentials_that_break_basic_auth() {
        assert!(
            config(&[
                "test",
                "--password",
                "some-long-secret",
                "--username",
                "bad:name"
            ])
            .validate()
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn x11_attachment_requires_local_display_and_does_not_mix_session_modes() {
        let base = ["test", "--password", "some-long-secret"];
        for display in ["localhost:0", "host:0", ":", ":0.", ":-1", ":1.2.3"] {
            let mut args = base.to_vec();
            args.extend(["--x11-display", display]);
            assert!(config(&args).validate().is_err(), "{display}");
        }
        for display in [":0", ":12.2"] {
            let mut args = base.to_vec();
            args.extend([
                "--x11-display",
                display,
                "--xauthority",
                "/tmp/test-Xauthority",
            ]);
            assert!(config(&args).validate().is_ok(), "{display}");
        }
        for conflicting in ["--wayland-socket", "--session-runtime"] {
            let mut args = base.to_vec();
            args.extend(["--x11-display", ":0", conflicting, "/tmp/other"]);
            assert!(Config::try_parse_from(args).is_err(), "{conflicting}");
        }
        assert!(
            config(&[
                "test",
                "--password",
                "some-long-secret",
                "--xauthority",
                "relative"
            ])
            .validate()
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn service_runtime_default_preserves_explicit_targets() {
        let default = PathBuf::from("/run/lrd-session");
        let mut owned = config(&["test", "--password", "some-long-secret"]);
        owned.apply_service_defaults(Some(default.clone()));
        assert_eq!(owned.session_runtime, Some(default.clone()));
        assert!(owned.validate().is_ok());

        for arguments in [
            vec![
                "test",
                "--password",
                "some-long-secret",
                "--session-runtime",
                "/run/custom",
            ],
            vec![
                "test",
                "--password",
                "some-long-secret",
                "--wayland-socket",
                "/run/user/1000/wayland-0",
            ],
            vec![
                "test",
                "--password",
                "some-long-secret",
                "--x11-display",
                ":0",
            ],
        ] {
            let mut settings = config(&arguments);
            let selected = settings.session_runtime.clone();
            settings.apply_service_defaults(Some(default.clone()));
            assert_eq!(settings.session_runtime, selected);
            assert!(settings.validate().is_ok());
        }

        let mut no_default = config(&["test", "--password", "some-long-secret"]);
        no_default.apply_service_defaults(None);
        assert!(no_default.session_runtime.is_none());
    }
}
use std::{net::IpAddr, path::PathBuf};

use anyhow::{Result, bail, ensure};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum DesktopShell {
    Auto,
    Minimal,
    Omarchy,
}

#[derive(Clone, Parser)]
pub struct Config {
    #[command(flatten)]
    pub rdp: crate::rdp::RdpConfig,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_ADDR", default_value = "0.0.0.0")]
    pub listen: IpAddr,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_PORT", default_value_t = 8766)]
    pub port: u16,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_AUTH_USER",
        default_value = "lightos"
    )]
    pub username: String,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_AUTH_PASSWORD",
        default_value = "",
        hide_env_values = true
    )]
    pub password: String,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_SUBFOLDER", default_value = "")]
    pub subfolder: String,
    /// Private MCP enable state and hashed credentials (defaults to the app state directory).
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_MCP_SETTINGS")]
    pub mcp_settings: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_SCREEN")]
    pub screen: Option<String>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_WIDTH", default_value_t = 1280)]
    pub width: u32,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_HEIGHT", default_value_t = 720)]
    pub height: u32,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_FPS", default_value_t = 30)]
    pub fps: u32,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_QUALITY", default_value_t = 75)]
    pub quality: u8,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_RUNTIME_DIR")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub runtime_dir: Option<PathBuf>,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_SESSION_RUNTIME",
        conflicts_with_all = ["wayland_socket", "x11_display"]
    )]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub session_runtime: Option<PathBuf>,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_WAYLAND_SOCKET",
        conflicts_with = "x11_display"
    )]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub wayland_socket: Option<PathBuf>,
    /// Attach to one local X11 display (for example, :0 or :0.0).
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_X11_DISPLAY",
        conflicts_with = "physical_wayland_socket"
    )]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub x11_display: Option<String>,
    /// Exact X11 authorization file; defaults to XAUTHORITY or ~/.Xauthority.
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_XAUTHORITY")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub xauthority: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_PHYSICAL_WAYLAND_SOCKET")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub physical_wayland_socket: Option<PathBuf>,
    /// Display protocol override, independent of distribution and desktop configuration flavor.
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_DISPLAY_BACKEND",
        default_value = "auto"
    )]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub display_backend: display_control::DisplayBackend,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_DESKTOP_CONFIG")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub desktop_config: Option<PathBuf>,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_SHELL",
        value_enum,
        default_value = "auto"
    )]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub desktop_shell: DesktopShell,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_CAGE_BIN", default_value = "cage")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub cage: PathBuf,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR")]
    #[arg(hide = !cfg!(target_os = "linux"))]
    pub aquamarine_dir: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_AUDIO", default_value_t = true, action = clap::ArgAction::Set)]
    pub audio: bool,
}

impl Config {
    /// The service-owned runtime is a fallback, never a competing target.
    pub fn apply_service_defaults(&mut self, default_runtime: Option<PathBuf>) {
        if self.session_runtime.is_none()
            && self.wayland_socket.is_none()
            && self.x11_display.is_none()
        {
            self.session_runtime = default_runtime;
        }
    }

    #[cfg(target_os = "linux")]
    pub fn omarchy_path(&self) -> Result<Option<PathBuf>> {
        if self.desktop_shell == DesktopShell::Minimal
            || self.desktop_config.is_some()
            || self.wayland_socket.is_some()
            || self.x11_display.is_some()
        {
            return Ok(None);
        }
        let installation = platform_detect::OmarchyInstallation::detect();
        ensure!(
            installation.is_some() || self.desktop_shell != DesktopShell::Omarchy,
            "Omarchy shell is not installed"
        );
        Ok(installation.map(|installation| installation.root))
    }

    pub fn dimensions(&self) -> Result<(u32, u32)> {
        let Some(screen) = &self.screen else {
            return Ok((self.width, self.height));
        };
        let mut parts = screen.split('x');
        let width = parts.next().unwrap_or_default().parse()?;
        let height = parts.next().unwrap_or_default().parse()?;
        Ok((width, height))
    }

    pub fn validate(&self) -> Result<()> {
        self.rdp.validate(
            !cfg!(target_os = "linux")
                || self.wayland_socket.is_some()
                || self.x11_display.is_some(),
        )?;
        #[cfg(not(target_os = "linux"))]
        ensure!(
            self.wayland_socket.is_none()
                && self.physical_wayland_socket.is_none()
                && self.x11_display.is_none()
                && self.xauthority.is_none()
                && self.runtime_dir.is_none()
                && self.session_runtime.is_none()
                && self.desktop_config.is_none()
                && self.aquamarine_dir.is_none()
                && self.desktop_shell == DesktopShell::Auto
                && self.display_backend == display_control::DisplayBackend::Auto
                && self.cage == PathBuf::from("cage"),
            "Linux desktop/session options are unsupported on this platform"
        );
        let (width, height) = self.dimensions()?;
        ensure!(
            (320..=3840).contains(&width) && (200..=2160).contains(&height),
            "invalid legacy screen size"
        );
        ensure!(
            self.subfolder.len() <= 1024
                && self
                    .subfolder
                    .trim_matches('/')
                    .split('/')
                    .all(|part| !matches!(part, "." | "..")
                        && part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte))),
            "invalid URL subfolder"
        );
        ensure!(
            (320..=3840).contains(&self.width),
            "width must be 320..3840"
        );
        ensure!(
            (200..=2160).contains(&self.height),
            "height must be 200..2160"
        );
        ensure!((1..=60).contains(&self.fps), "fps must be 1..60");
        ensure!((20..=95).contains(&self.quality), "quality must be 20..95");
        ensure!(self.port != 0, "port must not be zero");
        ensure!(
            !self.username.is_empty() && !self.username.contains([':', '\r', '\n']),
            "invalid Basic Auth username"
        );
        ensure!(self.username.len() <= 128, "username too long");
        ensure!(
            self.password.len() >= 12,
            "set an explicit password of at least 12 bytes"
        );
        if matches!(
            self.password.as_str(),
            "changeme" | "password" | "lightos" | "changeme123456" | "password123456"
        ) {
            bail!("refusing placeholder password");
        }
        ensure!(
            self.password.len() <= 4096 && !self.password.contains(['\r', '\n']),
            "invalid password"
        );
        if let Some(path) = &self.mcp_settings {
            ensure!(
                path.is_absolute() && path.file_name().is_some(),
                "MCP settings must be an absolute file path"
            );
        }
        if let Some(socket) = &self.wayland_socket {
            ensure!(
                socket.is_absolute(),
                "Wayland socket must be an absolute path"
            );
        }
        #[cfg(target_os = "linux")]
        if let Some(display) = &self.x11_display {
            crate::session::validate_local_x11_display(display)?;
        }
        if let Some(path) = &self.xauthority {
            ensure!(path.is_absolute(), "Xauthority must be an absolute path");
        }
        if let Some(socket) = &self.physical_wayland_socket {
            ensure!(
                socket.is_absolute(),
                "physical Wayland socket must be absolute"
            );
        }
        if let Some(directory) = &self.session_runtime {
            ensure!(
                directory.is_absolute(),
                "persistent session runtime must be absolute"
            );
        }
        if let Some(directory) = &self.runtime_dir {
            ensure!(
                directory.is_absolute(),
                "runtime directory must be absolute"
            );
            #[cfg(target_os = "linux")]
            crate::session::validate_runtime_socket_paths(directory)?;
        }
        Ok(())
    }
}
