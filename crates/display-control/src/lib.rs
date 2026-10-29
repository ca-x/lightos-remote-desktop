//! Session-scoped output discovery and control, independent of distro and transport.
#[cfg(target_os = "linux")]
mod command;
mod controller;
#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod kscreen;
#[cfg(target_os = "linux")]
mod mutter;
#[cfg(target_os = "linux")]
mod nested;
#[cfg(target_os = "linux")]
mod session;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod wlroots;
#[cfg(target_os = "linux")]
mod x11;

pub use controller::DisplayBackend;
#[cfg(target_os = "linux")]
pub use hyprland::HyprlandDisplay;
#[cfg(target_os = "linux")]
pub use kscreen::KScreenDisplay;
#[cfg(target_os = "linux")]
pub use mutter::MutterDisplay;
#[cfg(target_os = "linux")]
pub use session::{DesktopEnvironment, SessionTarget};
#[cfg(target_os = "linux")]
pub use wayland::WaylandDisplay;
#[cfg(target_os = "linux")]
pub use wlroots::WlrootsDisplay;
#[cfg(target_os = "linux")]
pub use x11::X11Display;

use anyhow::{Result, ensure};
use futures_util::future::BoxFuture;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct DisplayOutput {
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

impl DisplayMode {
    pub const SUPPORTED_SCALES: [f64; 9] = [1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 2.75, 3.0];

    pub fn validate(self) -> Result<()> {
        ensure!(
            (320..=3840).contains(&self.width) && (200..=2160).contains(&self.height),
            "invalid dimensions"
        );
        ensure!(
            Self::SUPPORTED_SCALES.contains(&self.scale),
            "invalid desktop scale"
        );
        let width = self.width as f64 / self.scale;
        let height = self.height as f64 / self.scale;
        ensure!(
            width.fract() == 0.0 && height.fract() == 0.0 && width >= 320.0 && height >= 200.0,
            "invalid scaled dimensions"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DisplayCapabilities {
    pub resize: bool,
    pub resize_scale: bool,
    pub reason: Option<String>,
}

impl DisplayCapabilities {
    pub fn unavailable(reason: impl ToString) -> Self {
        Self {
            resize: false,
            resize_scale: false,
            reason: Some(reason.to_string()),
        }
    }
}

pub trait DisplayController: Send + Sync {
    /// Blocking, bounded query; callers on an async executor should use spawn_blocking.
    fn outputs(&self) -> Result<Vec<DisplayOutput>>;
    /// Read-only probe against this session, never against ambient desktop variables.
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities>;
    fn resize<'a>(
        &'a self,
        output: Option<&'a str>,
        resolution: DisplayMode,
        fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>>;
}

#[cfg(target_os = "linux")]
pub(crate) fn select_output<'a>(
    outputs: &'a [DisplayOutput],
    name: Option<&str>,
) -> Result<&'a DisplayOutput> {
    match name {
        Some(name) => outputs
            .iter()
            .find(|o| o.name == name)
            .ok_or_else(|| anyhow::anyhow!("selected output unavailable")),
        None if outputs.len() == 1 => Ok(&outputs[0]),
        _ => anyhow::bail!("select an output before resizing a multi-output desktop"),
    }
}
#[cfg(target_os = "linux")]
pub(crate) fn output_matches(output: &DisplayOutput, mode: DisplayMode) -> bool {
    output.width == mode.width
        && output.height == mode.height
        && (output.scale - mode.scale).abs() < 0.001
}
#[cfg(test)]
mod mode_tests {
    use super::*;
    #[test]
    fn high_density_modes_preserve_logical_dimensions() {
        for (width, height, scale) in [
            (780, 1476, 2.0),
            (582, 1110, 1.5),
            (801, 601, 1.0),
            (970, 1840, 2.5),
            (1080, 1602, 3.0),
            (1600, 1000, 1.25),
            (2240, 1400, 1.75),
            (2880, 1800, 2.25),
            (3520, 2112, 2.75),
        ] {
            assert!(
                DisplayMode {
                    width,
                    height,
                    scale
                }
                .validate()
                .is_ok()
            );
        }
        for (width, height, scale) in [
            (780, 1476, 0.0),
            (780, 1476, 4.0),
            (781, 1476, 2.0),
            (320, 200, 2.0),
            (4000, 2000, 2.0),
            (800, 600, f64::NAN),
            (1600, 1000, 1.2),
            (1602, 1000, 1.25),
        ] {
            assert!(
                DisplayMode {
                    width,
                    height,
                    scale
                }
                .validate()
                .is_err()
            );
        }
    }
}
