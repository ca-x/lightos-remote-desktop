//! Optional TLS RDP access to the existing owned virtual desktop.
use std::{net::SocketAddr, path::PathBuf};

use anyhow::{Result, ensure};
use clap::Args;

#[derive(Clone, Args)]
pub struct RdpConfig {
    /// Expose the current virtual desktop over authenticated TLS RDP (requires the rdp feature).
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_RDP", default_value_t = false)]
    pub rdp: bool,
    /// Private persistent RDP settings (defaults to the service user state directory).
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_RDP_SETTINGS")]
    pub rdp_settings: Option<PathBuf>,
    #[arg(
        long,
        env = "LIGHTOS_REMOTE_DESKTOP_RDP_LISTEN",
        default_value = "127.0.0.1:3389"
    )]
    pub rdp_listen: SocketAddr,
    /// PEM certificate chain for the RDP listener; clients must verify this identity.
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_RDP_TLS_CERT")]
    pub rdp_tls_cert: Option<PathBuf>,
    /// PEM private key matching the RDP certificate.
    #[arg(long, env = "LIGHTOS_REMOTE_DESKTOP_RDP_TLS_KEY")]
    pub rdp_tls_key: Option<PathBuf>,
}

impl RdpConfig {
    pub fn validate(&self, external_desktop: bool) -> Result<()> {
        if let Some(path) = &self.rdp_settings {
            ensure!(
                path.is_absolute() && path.file_name().is_some(),
                "RDP settings must be an absolute file path"
            );
        }
        if !self.rdp {
            return Ok(());
        }
        ensure!(
            cfg!(feature = "rdp"),
            "RDP is not compiled in; build with --features rdp (Rust 1.89+)"
        );
        ensure!(
            !external_desktop,
            "RDP is available only for the owned virtual desktop; external/physical sessions are read-only"
        );
        ensure!(self.rdp_listen.port() != 0, "RDP port must not be zero");
        ensure!(
            self.rdp_tls_cert.is_some() == self.rdp_tls_key.is_some(),
            "Specify both --rdp-tls-cert and --rdp-tls-key, or neither for automatic TLS"
        );
        Ok(())
    }
}

pub mod settings;

#[cfg(feature = "rdp")]
mod service;
#[cfg(feature = "rdp")]
pub use service::{RdpService, start};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use clap::Parser;

    #[test]
    fn rdp_is_disabled_and_loopback_by_default() {
        let config = Config::try_parse_from(["test", "--password", "some-long-secret"]).unwrap();
        assert!(!config.rdp.rdp);
        assert!(config.rdp.rdp_listen.ip().is_loopback());
        assert_eq!(config.rdp.rdp_listen.port(), 3389);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn enabling_rdp_requires_feature_owned_desktop_and_complete_explicit_tls() {
        let mut config =
            Config::try_parse_from(["test", "--password", "some-long-secret", "--rdp"]).unwrap();
        assert_eq!(
            config.validate().is_ok(),
            cfg!(all(feature = "rdp", target_os = "linux"))
        );
        config.rdp.rdp_tls_cert = Some(PathBuf::from("cert.pem"));
        assert!(config.validate().is_err());
        config.rdp.rdp_tls_key = Some(PathBuf::from("key.pem"));
        assert_eq!(
            config.validate().is_ok(),
            cfg!(all(feature = "rdp", target_os = "linux"))
        );
        config.wayland_socket = Some(PathBuf::from("/tmp/physical-wayland"));
        assert!(config.validate().is_err());
        config.wayland_socket = None;
        config.rdp.rdp_listen.set_port(0);
        assert!(config.validate().is_err());
    }
}
