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
    pub runtime_dir: Option<PathBuf>,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_SESSION_RUNTIME",
        conflicts_with = "wayland_socket"
    )]
    pub session_runtime: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_WAYLAND_SOCKET")]
    pub wayland_socket: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_PHYSICAL_WAYLAND_SOCKET")]
    pub physical_wayland_socket: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_DESKTOP_CONFIG")]
    pub desktop_config: Option<PathBuf>,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_SHELL",
        value_enum,
        default_value = "auto"
    )]
    pub desktop_shell: DesktopShell,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_CAGE_BIN", default_value = "cage")]
    pub cage: PathBuf,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_AQUAMARINE_DIR")]
    pub aquamarine_dir: Option<PathBuf>,
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_AUDIO", default_value_t = true, action = clap::ArgAction::Set)]
    pub audio: bool,
}

impl Config {
    pub fn omarchy_path(&self) -> Result<Option<PathBuf>> {
        if self.desktop_shell == DesktopShell::Minimal
            || self.desktop_config.is_some()
            || self.wayland_socket.is_some()
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
        if let Some(socket) = &self.wayland_socket {
            ensure!(
                socket.is_absolute(),
                "Wayland socket must be an absolute path"
            );
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
            ensure!(
                directory.as_os_str().len() <= 28,
                "runtime directory path must fit Hyprland IPC sockets (max 28 bytes)"
            );
        }
        Ok(())
    }
}
