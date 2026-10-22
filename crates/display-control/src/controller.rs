use crate::{
    DesktopEnvironment, DisplayCapabilities, DisplayController, DisplayMode, DisplayOutput,
    HyprlandDisplay, KScreenDisplay, MutterDisplay, SessionTarget, WaylandDisplay, WlrootsDisplay,
    X11Display,
};
use anyhow::{Result, bail};
use futures_util::future::BoxFuture;
use std::{str::FromStr, sync::Arc};

/// Protocol selection is independent of distribution and configuration initialization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayBackend {
    #[default]
    Auto,
    Hyprland,
    Mutter,
    Kscreen,
    Wlroots,
    Wayland,
    X11,
}
impl FromStr for DisplayBackend {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s {
            "auto" => Ok(Self::Auto),
            "hyprland" => Ok(Self::Hyprland),
            "mutter" => Ok(Self::Mutter),
            "kscreen" => Ok(Self::Kscreen),
            "wlroots" => Ok(Self::Wlroots),
            "wayland" => Ok(Self::Wayland),
            "x11" => Ok(Self::X11),
            _ => Err("expected auto, hyprland, mutter, kscreen, wlroots, wayland or x11".into()),
        }
    }
}
impl DisplayBackend {
    pub fn controller(
        self,
        target: SessionTarget,
        hint: DesktopEnvironment,
        signature: Option<&str>,
        owned: bool,
    ) -> Result<Arc<dyn DisplayController>> {
        let backend = match self {
            Self::Auto if signature.is_some() => Self::Hyprland,
            Self::Auto => match hint {
                DesktopEnvironment::Gnome => Self::Mutter,
                DesktopEnvironment::Kde => Self::Kscreen,
                DesktopEnvironment::X11 => Self::X11,
                _ => Self::Wlroots,
            },
            other => other,
        };
        let fallback_socket = target.wayland_socket.clone();
        let controller: Arc<dyn DisplayController> = match backend {
            Self::Hyprland if !owned && signature.is_none() => Arc::new(WaylandDisplay::new(
                target
                    .wayland_socket
                    .ok_or_else(|| anyhow::anyhow!("missing Wayland socket"))?,
            )),
            Self::Hyprland => Arc::new(HyprlandDisplay::new(
                target
                    .wayland_socket
                    .ok_or_else(|| anyhow::anyhow!("missing Wayland socket"))?,
                target.runtime,
                signature.ok_or_else(|| anyhow::anyhow!("missing selected Hyprland signature"))?,
            )?),
            Self::Mutter => Arc::new(MutterDisplay::new(target)),
            Self::Kscreen => Arc::new(KScreenDisplay::new(target)),
            Self::Wlroots => Arc::new(WlrootsDisplay::new(
                target
                    .wayland_socket
                    .ok_or_else(|| anyhow::anyhow!("missing Wayland socket"))?,
            )),
            Self::X11 => Arc::new(X11Display::new(target)),
            Self::Wayland => Arc::new(WaylandDisplay::new(
                target
                    .wayland_socket
                    .ok_or_else(|| anyhow::anyhow!("missing Wayland socket"))?,
            )),
            Self::Auto => unreachable!(),
        };
        let controller: Arc<dyn DisplayController> =
            if self == Self::Auto && backend != Self::Hyprland {
                if let Some(socket) = fallback_socket {
                    Arc::new(QueryFallback {
                        primary: controller,
                        fallback: WaylandDisplay::new(socket),
                    })
                } else {
                    controller
                }
            } else {
                controller
            };
        Ok(if owned {
            controller
        } else {
            Arc::new(ReadOnly(controller))
        })
    }
}
// Query fallback preserves output enumeration when an optional management protocol is absent.
struct QueryFallback {
    primary: Arc<dyn DisplayController>,
    fallback: WaylandDisplay,
}
impl DisplayController for QueryFallback {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        self.primary.outputs().or_else(|primary| {
            self.fallback.outputs().map_err(|fallback| {
                anyhow::anyhow!("display backend: {primary:#}; Wayland query: {fallback:#}")
            })
        })
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        self.primary.capabilities()
    }
    fn resize<'a>(
        &'a self,
        name: Option<&'a str>,
        mode: DisplayMode,
        fps: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        self.primary.resize(name, mode, fps)
    }
}
/// Authorization belongs to the session, rather than to desktop or distribution detection.
struct ReadOnly(Arc<dyn DisplayController>);
impl DisplayController for ReadOnly {
    fn outputs(&self) -> Result<Vec<DisplayOutput>> {
        self.0.outputs()
    }
    fn capabilities(&self) -> BoxFuture<'_, DisplayCapabilities> {
        Box::pin(async {
            DisplayCapabilities::unavailable("resize requires an owned desktop session")
        })
    }
    fn resize<'a>(
        &'a self,
        _: Option<&'a str>,
        _: DisplayMode,
        _: u32,
    ) -> BoxFuture<'a, Result<DisplayMode>> {
        Box::pin(async { bail!("resize requires an owned desktop session") })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_hyprland_readonly_query_does_not_require_owned_signature() {
        let target = SessionTarget {
            wayland_socket: Some("/nonexistent/physical/wayland".into()),
            ..Default::default()
        };
        let controller=DisplayBackend::Hyprland.controller(target,DesktopEnvironment::Hyprland,None,false)
            .expect("read-only output discovery needs a Wayland socket, not a private Hyprland signature");
        assert!(!controller.capabilities().await.resize);
        assert!(
            controller
                .resize(
                    None,
                    DisplayMode {
                        width: 800,
                        height: 600,
                        scale: 1.0
                    },
                    30
                )
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn physical_authorization_blocks_every_backend_before_any_probe_or_write() {
        for backend in [
            DisplayBackend::Mutter,
            DisplayBackend::Kscreen,
            DisplayBackend::X11,
            DisplayBackend::Wlroots,
        ] {
            let target = SessionTarget {
                wayland_socket: Some("/nonexistent/wayland".into()),
                ..Default::default()
            };
            let controller = backend
                .controller(target, DesktopEnvironment::Unknown, None, false)
                .unwrap();
            assert!(!controller.capabilities().await.resize);
            assert!(
                controller
                    .resize(
                        None,
                        DisplayMode {
                            width: 800,
                            height: 600,
                            scale: 1.0
                        },
                        30
                    )
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("owned desktop")
            );
        }
    }
}
