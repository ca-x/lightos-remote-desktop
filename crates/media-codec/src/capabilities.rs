//! Shared capability discovery. Hardware work never blocks a desktop handshake.
use crate::{CodecCapability, EncoderFactory, HardwareEncoders, SoftwareEncoders};
use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{OnceCell, watch};

type Identity = Vec<(PathBuf, Option<String>)>;
const FAILURE_TTL: Duration = Duration::from_secs(600);
const SUCCESS_TTL: Duration = Duration::from_secs(3600);

struct Cached {
    identity: Identity,
    expires: Instant,
    capabilities: Vec<CodecCapability>,
}
#[derive(Default)]
struct HardwareState {
    identity: Identity,
    cached: Option<Cached>,
    running: bool,
}

pub struct CapabilityCache {
    software: OnceCell<Vec<CodecCapability>>,
    hardware: Mutex<HardwareState>,
    changed: watch::Sender<u64>,
}
impl Default for CapabilityCache {
    fn default() -> Self {
        Self {
            software: OnceCell::new(),
            hardware: Mutex::new(HardwareState::default()),
            changed: watch::channel(0).0,
        }
    }
}
impl CapabilityCache {
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    pub fn snapshot(&self) -> Vec<CodecCapability> {
        let mut capabilities = self.software.get().cloned().unwrap_or_default();
        if let Some(cached) = &self.hardware.lock().unwrap().cached {
            capabilities.extend(cached.capabilities.clone());
        }
        capabilities
    }
    pub fn initialized(&self) -> bool {
        self.software.initialized() && self.hardware.lock().unwrap().cached.is_some()
    }
    pub fn initialized_snapshot(&self) -> Option<Vec<CodecCapability>> {
        let software = self.software.get()?;
        let hardware = self.hardware.lock().unwrap();
        let cached = hardware.cached.as_ref()?;
        Some(
            software
                .iter()
                .chain(cached.capabilities.iter())
                .cloned()
                .collect(),
        )
    }
    /// Only software's real test frame is on the handshake path. Native probes
    /// retain their full decode/rate checks and deadlines in a single background job.
    pub async fn capabilities(
        self: &Arc<Self>,
        hardware: HardwareEncoders,
    ) -> Vec<CodecCapability> {
        self.warm(hardware);
        self.software.get_or_init(|| SoftwareEncoders.probe()).await;
        self.snapshot()
    }
    pub fn warm(self: &Arc<Self>, hardware: HardwareEncoders) {
        self.warm_with(hardware.identity(), Instant::now(), async move {
            hardware.probe().await
        });
    }
    fn warm_with(
        self: &Arc<Self>,
        identity: Identity,
        now: Instant,
        probe: impl Future<Output = Vec<CodecCapability>> + Send + 'static,
    ) {
        let mut state = self.hardware.lock().unwrap();
        if state.identity != identity {
            state.identity = identity.clone();
            state.cached = None;
        }
        if state.running
            || state
                .cached
                .as_ref()
                .is_some_and(|cached| cached.identity == identity && now < cached.expires)
        {
            return;
        }
        state.running = true;
        drop(state);
        let cache = self.clone();
        tokio::spawn(async move {
            // Reset single-flight state even if the task unwinds.
            struct Flight(Arc<CapabilityCache>);
            impl Drop for Flight {
                fn drop(&mut self) {
                    self.0.hardware.lock().unwrap().running = false;
                }
            }
            let flight = Flight(cache.clone());
            let capabilities = probe.await;
            let failed = capabilities.iter().any(|cap| {
                cap.hardware
                    && !cap.available
                    && crate::EncoderBackend::HARDWARE
                        .iter()
                        .any(|backend| backend.id() == cap.backend && backend.supported_platform())
            });
            let mut state = cache.hardware.lock().unwrap();
            if state.identity == identity {
                state.cached = Some(Cached {
                    identity,
                    expires: Instant::now() + if failed { FAILURE_TTL } else { SUCCESS_TTL },
                    capabilities,
                });
            }
            drop(state);
            drop(flight);
            cache
                .changed
                .send_modify(|revision| *revision = revision.wrapping_add(1));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EncoderBackend, VideoCodec};
    use std::sync::atomic::{AtomicUsize, Ordering};
    fn key(name: &str) -> Identity {
        vec![(PathBuf::from(name), None)]
    }
    fn capability(available: bool) -> CodecCapability {
        CodecCapability {
            codec: VideoCodec::H264,
            backend: EncoderBackend::HARDWARE
                .into_iter()
                .find(|b| b.supported_platform())
                .unwrap()
                .id(),
            hardware: true,
            available,
            reason: None,
            dynamic_quality: false,
            dynamic_bitrate: available,
            codec_string: None,
            rate_update: "restart",
        }
    }
    #[tokio::test]
    async fn background_probe_is_single_flight_and_does_not_block_readiness() {
        let cache = Arc::new(CapabilityCache::default());
        cache.software.set(Vec::new()).unwrap();
        let gate = Arc::new(tokio::sync::Notify::new());
        let starts = Arc::new(AtomicUsize::new(0));
        let mut changed = cache.subscribe();
        for _ in 0..20 {
            let (gate, starts) = (gate.clone(), starts.clone());
            cache.warm_with(key("worker"), Instant::now(), async move {
                starts.fetch_add(1, Ordering::SeqCst);
                gate.notified().await;
                vec![capability(true)]
            });
        }
        tokio::task::yield_now().await;
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert!(cache.snapshot().is_empty());
        assert!(!cache.initialized());
        gate.notify_one();
        tokio::time::timeout(Duration::from_secs(1), changed.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(cache.initialized());
        assert_eq!(cache.snapshot().len(), 1);
    }
    #[tokio::test]
    async fn successful_probe_uses_the_longer_ttl_and_then_refreshes() {
        let cache = Arc::new(CapabilityCache::default());
        let mut changed = cache.subscribe();
        let now = Instant::now();
        cache.warm_with(key("worker"), now, async { vec![capability(true)] });
        changed.changed().await.unwrap();
        cache.warm_with(key("worker"), now + FAILURE_TTL * 2, async {
            panic!("a successful probe must outlive the failure TTL")
        });
        assert!(!cache.hardware.lock().unwrap().running);
        cache.warm_with(
            key("worker"),
            now + SUCCESS_TTL + Duration::from_secs(1),
            async { vec![capability(false)] },
        );
        changed.changed().await.unwrap();
        assert!(!cache.snapshot()[0].available);
    }

    #[tokio::test]
    async fn failed_probe_expires_and_changed_identity_discards_old_result() {
        let cache = Arc::new(CapabilityCache::default());
        let mut changed = cache.subscribe();
        let now = Instant::now();
        cache.warm_with(key("v1"), now, async { vec![capability(false)] });
        changed.changed().await.unwrap();
        cache.warm_with(key("v1"), now + FAILURE_TTL / 2, async {
            panic!("cached failure must not launch a probe")
        });
        assert!(!cache.hardware.lock().unwrap().running);
        cache.warm_with(
            key("v1"),
            now + FAILURE_TTL + Duration::from_secs(1),
            async { vec![capability(true)] },
        );
        changed.changed().await.unwrap();
        assert!(cache.snapshot()[0].available);
        let gate = Arc::new(tokio::sync::Notify::new());
        let release = gate.clone();
        cache.warm_with(key("v2"), Instant::now(), async move {
            release.notified().await;
            vec![capability(false)]
        });
        assert!(cache.snapshot().is_empty());
        cache.warm_with(key("v3"), Instant::now(), async {
            panic!("must not overlap v2")
        });
        gate.notify_one();
        changed.changed().await.unwrap();
        assert!(
            cache.snapshot().is_empty(),
            "a stale worker result must not be published"
        );
        cache.warm_with(key("v3"), Instant::now(), async { vec![capability(true)] });
        changed.changed().await.unwrap();
        assert!(cache.snapshot()[0].available);
    }
}
