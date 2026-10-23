//! Automatic backend selection within one already resolved desktop session.
use super::{BackendKind, DesktopBackend};
use anyhow::{Context, Result, bail, ensure};
use display_control::SessionTarget;
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Debug, Serialize)]
pub struct BackendAvailability {
    pub backend: BackendKind,
    pub available: bool,
    pub requires_consent: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BackendReport {
    /// A candidate is not ready until it has produced its first validated frame.
    pub selected: Option<BackendKind>,
    pub candidates: Vec<BackendAvailability>,
}
impl BackendReport {
    pub fn require_available(&self) -> Result<BackendKind> {
        self.selected.ok_or_else(|| {
            anyhow::anyhow!(
                "desktop capture/input unavailable: {}",
                self.candidates
                    .iter()
                    .filter_map(|candidate| candidate.reason.as_deref())
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        })
    }
    pub fn requires_consent(&self) -> bool {
        self.candidates
            .iter()
            .any(|candidate| Some(candidate.backend) == self.selected && candidate.requires_consent)
    }
}

fn candidate(
    backend: BackendKind,
    requires_consent: bool,
    result: Result<()>,
) -> BackendAvailability {
    BackendAvailability {
        backend,
        requires_consent,
        available: result.is_ok(),
        reason: result.err().map(|error| format!("{error:#}")),
    }
}
fn report(candidates: Vec<BackendAvailability>) -> BackendReport {
    BackendReport {
        selected: candidates
            .iter()
            .find(|candidate| candidate.available)
            .map(|candidate| candidate.backend),
        candidates,
    }
}

/// No consent prompts, input devices, clipboard ownership or display changes.
/// Run on a blocking worker: protocol probes perform bounded socket/D-Bus I/O.
pub fn probe(
    target: &SessionTarget,
    selected_output: bool,
    cancelled: Option<&AtomicBool>,
) -> Result<BackendReport> {
    ensure!(
        !cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)),
        "desktop detection cancelled"
    );
    if let Some(socket) = &target.wayland_socket {
        // DISPLAY may name XWayland on this very session. It is deliberately
        // excluded: its root window is not a capture of the Wayland desktop.
        let native = crate::wayland::capabilities::probe(socket, Duration::from_secs(1), cancelled)
            .and_then(|capabilities| {
                ensure!(
                    capabilities.capture
                        && capabilities.keyboard
                        && if selected_output {
                            capabilities.selected_output_pointer
                        } else {
                            capabilities.pointer
                        },
                    "native Wayland protocols missing: {}",
                    capabilities.missing.join(", ")
                );
                Ok(())
            });
        ensure!(
            !cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)),
            "desktop detection cancelled"
        );
        let mut candidates = vec![candidate(BackendKind::Wayland, false, native)];
        // A healthy native path is sufficient; discovery must not activate an
        // unrelated portal or pay for a second backend on every connection.
        if !candidates[0].available {
            candidates.push(candidate(
                BackendKind::Portal,
                true,
                super::portal::probe(target),
            ));
        }
        return Ok(report(candidates));
    }
    if target.x11_display.is_some() {
        return Ok(report(vec![candidate(
            BackendKind::X11,
            false,
            super::x11::probe(target),
        )]));
    }
    bail!("no explicit desktop session target")
}

/// Select once. Initialization errors, consent denial and cancellation are
/// returned to the caller rather than triggering another authorization path.
pub fn start_auto(
    target: SessionTarget,
    fps: u32,
    quality: u8,
    output: Option<String>,
    cancelled: Arc<AtomicBool>,
) -> Result<Arc<dyn DesktopBackend>> {
    let selection = probe(&target, output.is_some(), Some(&cancelled))?;
    let selected = selection.require_available()?;
    tracing::info!(
        ?selected,
        requires_consent = selection.requires_consent(),
        "desktop backend selected automatically"
    );
    start_selected(selected, target, fps, quality, output, cancelled)
}

/// Start the exact backend already selected for this target and output.
/// Re-probing after the Portal chooser has removed a named output can select
/// another capture source, so callers must preserve their probe decision.
pub fn start_selected(
    selected: BackendKind,
    target: SessionTarget,
    fps: u32,
    quality: u8,
    output: Option<String>,
    cancelled: Arc<AtomicBool>,
) -> Result<Arc<dyn DesktopBackend>> {
    ensure!((1..=60).contains(&fps), "FPS must be between 1 and 60");
    ensure!(
        (1..=100).contains(&quality),
        "JPEG quality must be between 1 and 100"
    );
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "desktop startup cancelled"
    );
    match selected {
        BackendKind::Wayland => Ok(Arc::new(
            crate::wayland::Backend::start_with_output_cancellable(
                target.wayland_socket.context("missing Wayland socket")?,
                fps,
                quality,
                output,
                cancelled,
            )?,
        )),
        BackendKind::X11 => {
            ensure!(
                target.wayland_socket.is_none() && target.x11_display.is_some(),
                "X11 requires an explicit X11-only target"
            );
            Ok(Arc::new(super::x11::Backend::start(
                target, fps, quality, output, cancelled,
            )?))
        }
        BackendKind::Portal => {
            ensure!(
                target.wayland_socket.is_some(),
                "Portal requires a selected Wayland socket"
            );
            Ok(Arc::new(super::portal::Backend::start(
                target, fps, quality, output, cancelled,
            )?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_order_selects_native_then_authorized_portal() {
        let native = candidate(BackendKind::Wayland, false, Ok(()));
        let portal = candidate(BackendKind::Portal, true, Ok(()));
        assert_eq!(
            report(vec![native, portal.clone()])
                .require_available()
                .unwrap(),
            BackendKind::Wayland
        );
        let unavailable = candidate(
            BackendKind::Wayland,
            false,
            Err(anyhow::anyhow!("no virtual keyboard")),
        );
        let fallback = report(vec![unavailable, portal]);
        assert_eq!(fallback.require_available().unwrap(), BackendKind::Portal);
        assert!(fallback.requires_consent());
    }
    #[test]
    fn selected_output_portal_choice_survives_a_different_default_probe() {
        let selected_output = report(vec![
            candidate(
                BackendKind::Wayland,
                false,
                Err(anyhow::anyhow!("pointer v1 cannot target a monitor")),
            ),
            candidate(BackendKind::Portal, true, Ok(())),
        ]);
        let default_output = report(vec![candidate(BackendKind::Wayland, false, Ok(()))]);
        assert_eq!(
            selected_output.require_available().unwrap(),
            BackendKind::Portal
        );
        assert_eq!(
            default_output.require_available().unwrap(),
            BackendKind::Wayland
        );
        assert!(selected_output.requires_consent());
        // The Portal route must remain selected after its chooser clears the
        // native output name. A mismatched target fails before any I/O rather
        // than re-probing a different backend.
        let cancelled = Arc::new(AtomicBool::new(false));
        let selected = selected_output.require_available().unwrap();
        let target = SessionTarget {
            x11_display: Some(":99".into()),
            ..Default::default()
        };
        assert!(
            start_selected(selected, target, 30, 75, None, cancelled)
                .err()
                .unwrap()
                .to_string()
                .contains("Portal requires a selected Wayland socket")
        );
    }
    #[test]
    fn failures_remain_explicit_instead_of_inventing_a_default() {
        let report = report(vec![candidate(
            BackendKind::X11,
            false,
            Err(anyhow::anyhow!("XTest unavailable")),
        )]);
        assert!(
            report
                .require_available()
                .unwrap_err()
                .to_string()
                .contains("XTest unavailable")
        );
        assert_eq!(report.selected, None);
    }
    #[test]
    fn cancelled_detection_performs_no_probe() {
        let cancelled = AtomicBool::new(true);
        let target = SessionTarget {
            wayland_socket: Some("/nonexistent".into()),
            x11_display: Some(":0".into()),
            ..Default::default()
        };
        assert!(
            probe(&target, true, Some(&cancelled))
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
    }
}
