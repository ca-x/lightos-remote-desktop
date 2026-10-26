//! Host lifecycle and native desktop capabilities, independent of transport.
use std::sync::{Arc, atomic::AtomicBool};

use anyhow::{Result, ensure};
use display_control::{DisplayController, DisplayOutput};
use serde::Serialize;

use crate::{
    audio::AudioCaptureFactory,
    backend::{BackendKind, BackendReport, DesktopBackend},
    config::Config,
};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod native;
#[cfg(target_os = "windows")]
pub mod windows;

pub trait SessionLifecycle: Send {
    fn healthy(&mut self) -> bool;
    fn is_owned(&self) -> bool;
}

pub struct HostSession {
    pub backend: Arc<dyn DesktopBackend>,
    pub displays: Arc<dyn DisplayController>,
    pub lifecycle: Box<dyn SessionLifecycle>,
    pub audio: Arc<dyn AudioCaptureFactory>,
    pub output: Option<String>,
    pub outputs: Vec<DisplayOutput>,
}

pub trait HostPlatform: Send + Sync {
    fn physical_displays(&self, config: &Config) -> Result<Arc<dyn DisplayController>>;
    fn physical_key(&self, config: &Config) -> Result<String>;
    fn probe_physical(
        &self,
        config: &Config,
        cancelled: Option<&AtomicBool>,
    ) -> Result<BackendReport>;
    fn supports_virtual(&self, config: &Config) -> bool;
    fn open(&self, config: &Config, cancelled: Arc<AtomicBool>) -> Result<HostSession>;
    fn open_physical(
        &self,
        config: &Config,
        output: Option<String>,
        cancelled: Arc<AtomicBool>,
        authorization: tokio::sync::oneshot::Sender<()>,
    ) -> Result<HostSession>;
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    NeedsUserAction,
}

#[derive(Clone, Debug, Serialize)]
pub struct Permissions {
    pub screen: PermissionState,
    pub input: PermissionState,
    pub reason: Option<String>,
}

impl Permissions {
    pub fn require_granted(&self) -> Result<()> {
        ensure!(
            matches!(self.screen, PermissionState::Granted)
                && matches!(self.input, PermissionState::Granted),
            "{}",
            self.reason
                .as_deref()
                .unwrap_or("screen capture and input permission required")
        );
        Ok(())
    }
}

pub trait PermissionProvider: Send + Sync {
    /// Read-only: must never trigger a permission prompt.
    fn permissions(&self) -> Result<Permissions>;
}

pub fn current() -> Arc<dyn HostPlatform> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::LinuxHost)
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(native::NativeHost::new(windows::provider()))
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(native::NativeHost::new(macos::provider()))
    }
}

/// Native capture delivers packed RGBA pixels; callers retain only the newest frame.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub trait NativeCapture: Send {
    fn next_frame(&mut self, timeout: std::time::Duration)
    -> Result<Option<crate::backend::Frame>>;
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub trait NativePlatform: PermissionProvider {
    fn probe_input(&self) -> Result<()> {
        native::probe_input()
    }
    fn kind(&self) -> BackendKind;
    fn session_key(&self) -> Result<String>;
    /// False means locked or an inaccessible secure desktop. Errors end this host.
    fn session_active(&self) -> Result<bool>;
    fn displays(&self) -> Result<Vec<DisplayOutput>>;
    fn capture(&self, output: &DisplayOutput, fps: u32) -> Result<Box<dyn NativeCapture>>;
    fn audio(&self) -> Arc<dyn AudioCaptureFactory>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_permissions_are_required_and_probe_does_not_request_consent() {
        for (screen, input) in [
            (PermissionState::Denied, PermissionState::Granted),
            (PermissionState::Granted, PermissionState::Denied),
            (PermissionState::NeedsUserAction, PermissionState::Granted),
        ] {
            assert!(
                Permissions {
                    screen,
                    input,
                    reason: Some("system grant required".into())
                }
                .require_granted()
                .is_err()
            );
        }
        assert!(
            Permissions {
                screen: PermissionState::Granted,
                input: PermissionState::Granted,
                reason: None
            }
            .require_granted()
            .is_ok()
        );
    }
}
