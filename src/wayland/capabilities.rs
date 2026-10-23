use std::{
    collections::BTreeMap,
    net::Shutdown,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

/// Preliminary protocol support; successful first capture remains authoritative.
#[derive(Clone, Debug, Serialize)]
pub struct NativeCapabilities {
    pub capture: bool,
    pub keyboard: bool,
    pub pointer: bool,
    pub selected_output_pointer: bool,
    pub missing: Vec<String>,
}

impl NativeCapabilities {
    pub fn from_globals<'a>(globals: impl IntoIterator<Item = (&'a str, u32)>) -> Self {
        let mut versions = BTreeMap::new();
        for (name, version) in globals {
            versions
                .entry(name)
                .and_modify(|present: &mut u32| *present = (*present).max(version))
                .or_insert(version);
        }
        let supports = |name: &str, required| versions.get(name).copied().unwrap_or(0) >= required;
        let mut missing = Vec::new();
        let mut require = |name: &str, version| {
            let present = supports(name, version);
            if !present {
                missing.push(format!("{name} >= {version}"));
            }
            present
        };

        // libwayshot 0.9 ignores output globals below v4 and binds xdg-output v3.
        let compositor = require("wl_compositor", 3);
        let shm = require("wl_shm", 1);
        let output = require("wl_output", 4);
        let xdg_output = require("zxdg_output_manager_v1", 3);
        // libwayshot selects ext as soon as its capture manager binds, even with wlr present.
        let copy = if supports("ext_image_copy_capture_manager_v1", 1) {
            require("ext_output_image_capture_source_manager_v1", 1)
        } else {
            require("zwlr_screencopy_manager_v1", 3)
        };
        let seat = require("wl_seat", 1);
        let keyboard = require("zwp_virtual_keyboard_manager_v1", 1) && seat;
        let pointer = require("zwlr_virtual_pointer_manager_v1", 1) && seat;
        // The selected-output input path calls create_virtual_pointer_with_output (v2).
        let selected_output_pointer = require("zwlr_virtual_pointer_manager_v1", 2) && seat;
        Self {
            capture: compositor && shm && output && xdg_output && copy,
            keyboard,
            pointer,
            selected_output_pointer,
            missing,
        }
    }

    pub fn require_selected_output(&self) -> Result<()> {
        ensure!(
            self.capture && self.keyboard && self.selected_output_pointer,
            "native Wayland capture/input unavailable; missing {}; GNOME/KDE sessions may require a Portal backend, which is not implemented",
            self.missing.join(", ")
        );
        Ok(())
    }
}

/// Query an explicit Wayland socket without binding globals or creating input devices.
/// Closing the connection and joining its worker also bounds a silent registry exchange.
pub fn probe(
    socket: &Path,
    timeout: Duration,
    cancelled: Option<&AtomicBool>,
) -> Result<NativeCapabilities> {
    let deadline = Instant::now() + timeout;
    let stream = super::connect(socket, deadline, cancelled)?;
    let shutdown = stream.try_clone()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("wayland-capabilities".into())
        .spawn(move || {
            let report = (|| {
                let connection = wayland_client::Connection::from_socket(stream)?;
                let (globals, _queue) =
                    wayland_client::globals::registry_queue_init::<super::InputState>(&connection)?;
                Ok(globals.contents().with_list(|list| {
                    NativeCapabilities::from_globals(
                        list.iter()
                            .map(|global| (global.interface.as_str(), global.version)),
                    )
                }))
            })();
            let _ = sender.send(report);
        })
        .context("spawn Wayland capability probe")?;
    let result = loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            break Err(anyhow::anyhow!("Wayland capability probe cancelled"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err(anyhow::anyhow!("Wayland capability probe timed out"));
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(20))) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!("Wayland capability worker stopped"));
            }
        }
    };
    let _ = shutdown.shutdown(Shutdown::Both);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("Wayland capability worker panicked"))?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::net::{UnixListener, UnixStream},
    };

    fn wlr() -> Vec<(&'static str, u32)> {
        vec![
            ("wl_compositor", 3),
            ("wl_shm", 1),
            ("wl_output", 4),
            ("zxdg_output_manager_v1", 3),
            ("zwlr_screencopy_manager_v1", 3),
            ("wl_seat", 1),
            ("zwp_virtual_keyboard_manager_v1", 1),
            ("zwlr_virtual_pointer_manager_v1", 2),
        ]
    }

    #[test]
    fn complete_wlr_supports_capture_and_selected_output_input() {
        let report = NativeCapabilities::from_globals(wlr());
        assert!(
            report.capture && report.keyboard && report.pointer && report.selected_output_pointer
        );
        assert!(report.missing.is_empty());
        assert_eq!(serde_json::to_value(&report).unwrap()["capture"], true);
    }

    #[test]
    fn ext_capture_takes_precedence_over_wlr_and_needs_output_source() {
        let mut globals = wlr();
        globals.push(("ext_image_copy_capture_manager_v1", 1));
        let report = NativeCapabilities::from_globals(globals.clone());
        assert!(!report.capture);
        assert!(
            report
                .missing
                .iter()
                .any(|s| s.contains("ext_output_image_capture_source_manager_v1"))
        );
        globals.push(("ext_output_image_capture_source_manager_v1", 1));
        globals.retain(|(name, _)| *name != "zwlr_screencopy_manager_v1");
        let report = NativeCapabilities::from_globals(globals);
        assert!(report.capture);
        assert!(report.require_selected_output().is_ok());
    }

    #[test]
    fn pointer_v1_does_not_support_selected_monitor() {
        let mut globals = wlr();
        globals
            .iter_mut()
            .find(|(name, _)| *name == "zwlr_virtual_pointer_manager_v1")
            .unwrap()
            .1 = 1;
        let report = NativeCapabilities::from_globals(globals);
        assert!(report.capture && report.keyboard && report.pointer);
        assert!(!report.selected_output_pointer);
        assert!(
            report
                .require_selected_output()
                .unwrap_err()
                .to_string()
                .contains("zwlr_virtual_pointer_manager_v1 >= 2")
        );
    }

    #[test]
    fn missing_or_old_required_globals_are_not_usable() {
        for required in [
            "wl_compositor",
            "wl_shm",
            "wl_output",
            "zxdg_output_manager_v1",
            "zwlr_screencopy_manager_v1",
            "wl_seat",
            "zwp_virtual_keyboard_manager_v1",
            "zwlr_virtual_pointer_manager_v1",
        ] {
            let mut globals = wlr();
            let (_, version) = globals
                .iter_mut()
                .find(|(name, _)| *name == required)
                .unwrap();
            *version -= 1;
            let report = NativeCapabilities::from_globals(globals);
            assert!(report.require_selected_output().is_err(), "{required}");
            assert!(
                report.missing.iter().any(|s| s.starts_with(required)),
                "{report:?}"
            );
        }
        assert!(
            NativeCapabilities::from_globals([])
                .require_selected_output()
                .is_err()
        );
    }

    // Minimal Wayland wire fixture: wl_display.get_registry + sync, followed by
    // wl_registry.global and wl_callback.done. No capture/input objects are created.
    fn request(stream: &mut UnixStream) -> (u16, u32) {
        let mut header = [0; 8];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(u32::from_ne_bytes(header[..4].try_into().unwrap()), 1);
        let descriptor = u32::from_ne_bytes(header[4..].try_into().unwrap());
        let size = (descriptor >> 16) as usize;
        assert!((8..=4096).contains(&size));
        let mut payload = vec![0; size - 8];
        stream.read_exact(&mut payload).unwrap();
        assert_eq!(payload.len(), 4);
        (
            descriptor as u16,
            u32::from_ne_bytes(payload.try_into().unwrap()),
        )
    }

    fn event(stream: &mut UnixStream, object: u32, opcode: u16, payload: &[u8]) {
        stream.write_all(&object.to_ne_bytes()).unwrap();
        stream
            .write_all(&(((payload.len() as u32 + 8) << 16) | u32::from(opcode)).to_ne_bytes())
            .unwrap();
        stream.write_all(payload).unwrap();
    }

    fn registry_fixture(stream: &mut UnixStream, globals: &[(&str, u32)]) {
        let (opcode, registry) = request(stream);
        assert_eq!(opcode, 1, "wl_display.get_registry");
        let (opcode, callback) = request(stream);
        assert_eq!(opcode, 0, "wl_display.sync");
        for (index, &(interface, version)) in globals.iter().enumerate() {
            let mut payload = Vec::new();
            payload.extend_from_slice(&(index as u32 + 1).to_ne_bytes());
            payload.extend_from_slice(&(interface.len() as u32 + 1).to_ne_bytes());
            payload.extend_from_slice(interface.as_bytes());
            payload.push(0);
            payload.resize(payload.len().next_multiple_of(4), 0);
            payload.extend_from_slice(&version.to_ne_bytes());
            event(stream, registry, 0, &payload);
        }
        event(stream, callback, 0, &1_u32.to_ne_bytes());
    }

    #[test]
    fn live_registry_response_reports_wlr_ext_and_old_protocols() {
        for (globals, expected) in [
            (wlr(), true),
            (
                {
                    let mut ext = wlr();
                    ext.retain(|(name, _)| *name != "zwlr_screencopy_manager_v1");
                    ext.extend([
                        ("ext_image_copy_capture_manager_v1", 1),
                        ("ext_output_image_capture_source_manager_v1", 1),
                    ]);
                    ext
                },
                true,
            ),
            (
                {
                    let mut incomplete = wlr();
                    incomplete.push(("ext_image_copy_capture_manager_v1", 1));
                    incomplete
                },
                false,
            ),
            (
                {
                    let mut old = wlr();
                    old.iter_mut()
                        .find(|(name, _)| *name == "zxdg_output_manager_v1")
                        .unwrap()
                        .1 = 2;
                    old
                },
                false,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let socket = directory.path().join("wayland-fixture");
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let mut stream = listener.accept().unwrap().0;
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                registry_fixture(&mut stream, &globals);
                let mut bytes = Vec::new();
                stream
                    .read_to_end(&mut bytes)
                    .expect("probe must close its connection");
            });
            let report = probe(&socket, Duration::from_secs(1), None).unwrap();
            assert_eq!(
                report.require_selected_output().is_ok(),
                expected,
                "{report:?}"
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn silent_compositor_times_out_closes_socket_and_joins() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-silent");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            stream
                .read_to_end(&mut bytes)
                .expect("probe must close its socket");
            assert!(!bytes.is_empty(), "must actually query the registry");
        });
        let started = Instant::now();
        let error = probe(&socket, Duration::from_millis(60), None).unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }

    #[test]
    fn disconnected_compositor_releases_probe_worker() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-disconnect");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            assert_eq!(request(&mut stream).0, 1);
        });
        let started = Instant::now();
        assert!(probe(&socket, Duration::from_secs(1), None).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }

    #[test]
    fn cancellation_closes_silent_connection_before_deadline() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("wayland-cancel");
        let listener = UnixListener::bind(&socket).unwrap();
        let (connected, accepted) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap().0;
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            connected.send(()).unwrap();
            let mut bytes = Vec::new();
            stream
                .read_to_end(&mut bytes)
                .expect("cancellation must close the connection");
        });
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let probe_cancelled = cancelled.clone();
        let probe_thread = std::thread::spawn(move || {
            probe(&socket, Duration::from_secs(1), Some(&probe_cancelled))
        });
        accepted.recv_timeout(Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        cancelled.store(true, Ordering::Release);
        let error = probe_thread.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error:#}");
        assert!(started.elapsed() < Duration::from_millis(500));
        server.join().unwrap();
    }
}
