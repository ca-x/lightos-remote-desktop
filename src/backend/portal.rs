//! Optional Portal backend. Builds without PipeWire report an explicit reason.
#[cfg(feature = "portal")]
mod identity;
#[cfg(feature = "portal")]
mod native;
#[cfg(feature = "portal")]
pub use native::*;

#[cfg(not(feature = "portal"))]
mod unavailable {
    use super::super::{BackendKind, DesktopBackend, Frame, InputCommand};
    use anyhow::{Result, bail};
    use display_control::SessionTarget;
    use std::{
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };
    use tokio::sync::{mpsc, watch};
    pub fn probe(_target: &SessionTarget) -> Result<()> {
        bail!("Portal backend was not compiled; enable the portal feature")
    }
    pub struct Backend;
    impl Backend {
        pub fn start(
            _target: SessionTarget,
            _fps: u32,
            _quality: u8,
            _output: Option<String>,
            _cancelled: Arc<AtomicBool>,
        ) -> Result<Self> {
            bail!("Portal backend was not compiled; enable the portal feature")
        }
    }
    impl DesktopBackend for Backend {
        fn kind(&self) -> BackendKind {
            BackendKind::Portal
        }
        fn frames(&self) -> watch::Receiver<Option<Arc<Frame>>> {
            watch::channel(None).1
        }
        fn input(&self) -> mpsc::Sender<InputCommand> {
            mpsc::channel(1).0
        }
        fn stop(&self) {}
        fn frame_is_fresh(&self, _: Duration) -> bool {
            false
        }
    }
}
#[cfg(not(feature = "portal"))]
pub use unavailable::*;
