//! Session identity is independent of Linux distribution and configuration flavor.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEnvironment {
    Gnome,
    Kde,
    Hyprland,
    Wlroots,
    X11,
    Unknown,
}
impl DesktopEnvironment {
    /// A hint only. Backends must still probe their actual protocol.
    pub fn from_hint(desktop: &str, session_type: &str) -> Self {
        if session_type.eq_ignore_ascii_case("x11") {
            return Self::X11;
        }
        let names: Vec<_> = desktop.split(':').map(str::to_ascii_lowercase).collect();
        if names.iter().any(|n| n == "gnome") {
            Self::Gnome
        } else if names.iter().any(|n| n == "kde" || n == "plasma") {
            Self::Kde
        } else if names.iter().any(|n| n == "hyprland") {
            Self::Hyprland
        } else if names
            .iter()
            .any(|n| matches!(n.as_str(), "sway" | "river" | "wayfire" | "labwc"))
        {
            Self::Wlroots
        } else {
            Self::Unknown
        }
    }
}

/// All connection coordinates belong to the selected session; no ambient display/bus fallback.
#[derive(Clone, Debug, Default)]
pub struct SessionTarget {
    pub runtime: PathBuf,
    pub wayland_socket: Option<PathBuf>,
    pub bus_address: Option<String>,
    pub x11_display: Option<String>,
    pub xauthority: Option<PathBuf>,
}

impl SessionTarget {
    pub fn wayland(socket: PathBuf, runtime: PathBuf) -> Self {
        // Derive the bus solely from this session runtime, never from the transport's ambient bus.
        let bus_address = runtime.join("bus").exists().then(|| {
            zbus::address::transport::Unix::new(zbus::address::transport::UnixSocket::File(
                runtime.join("bus"),
            ))
            .to_string()
        });
        Self {
            runtime,
            wayland_socket: Some(socket),
            bus_address,
            ..Default::default()
        }
    }
    pub fn matching_environment_hint(&self) -> DesktopEnvironment {
        // Environment hints may only describe the exact selected socket.
        let Some(display) = std::env::var_os("WAYLAND_DISPLAY") else {
            return DesktopEnvironment::Unknown;
        };
        let display = PathBuf::from(display);
        let selected = if display.is_absolute() {
            display
        } else {
            let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
                return DesktopEnvironment::Unknown;
            };
            PathBuf::from(runtime).join(display)
        };
        if self.wayland_socket.as_ref() != Some(&selected) {
            return DesktopEnvironment::Unknown;
        }
        DesktopEnvironment::from_hint(
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            "wayland",
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_hints_do_not_identify_a_distribution() {
        assert_eq!(
            DesktopEnvironment::from_hint("GNOME", "wayland"),
            DesktopEnvironment::Gnome
        );
        assert_eq!(
            DesktopEnvironment::from_hint("KDE", "wayland"),
            DesktopEnvironment::Kde
        );
        assert_eq!(
            DesktopEnvironment::from_hint("Hyprland", "wayland"),
            DesktopEnvironment::Hyprland
        );
        for distro in ["arch", "ubuntu", "fedora", "manjaro", "omarchy"] {
            assert_eq!(
                DesktopEnvironment::from_hint(distro, "wayland"),
                DesktopEnvironment::Unknown
            );
        }
        assert_eq!(
            DesktopEnvironment::from_hint("XFCE", "x11"),
            DesktopEnvironment::X11
        );
    }
}
