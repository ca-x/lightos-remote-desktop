//! Network pacing and latency measurements, independent of codec and platform.
mod qos;
use qos::AdaptiveRate;
pub use qos::RateTarget;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub struct LatencySamples {
    samples: VecDeque<u32>,
}

impl LatencySamples {
    pub fn record(&mut self, elapsed: Duration) {
        if self.samples.len() == 120 {
            self.samples.pop_front();
        }
        self.samples
            .push_back(elapsed.as_micros().min(u32::MAX as u128) as u32);
    }

    pub fn summary(&self) -> serde_json::Value {
        let mut samples: Vec<_> = self.samples.iter().copied().collect();
        samples.sort_unstable();
        let percentile = |percent: usize| {
            if samples.is_empty() {
                None
            } else {
                Some(samples[(samples.len() * percent).div_ceil(100) - 1] as f64 / 1000.0)
            }
        };
        json!({"count":samples.len(),"p50_ms":percentile(50),"p95_ms":percentile(95)})
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulingStrategy {
    #[default]
    Auto,
    LowLatency,
    Smooth,
    QualityFirst,
}

#[derive(Clone)]
pub struct FrameScheduler {
    strategy: SchedulingStrategy,
    enabled: bool,
    sequenced: bool,
    pending: VecDeque<(u64, Instant)>,
    window: usize,
    frame_interval: Duration,
    dimensions: Option<(u32, u32)>,
    ack_samples: LatencySamples,
    qos: AdaptiveRate,
    last_sent: Option<Instant>,
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self {
            strategy: SchedulingStrategy::Auto,
            enabled: false,
            sequenced: false,
            pending: VecDeque::new(),
            window: 1,
            frame_interval: Duration::from_secs_f64(1.0 / 30.0),
            dimensions: None,
            ack_samples: LatencySamples::default(),
            qos: AdaptiveRate::new(30, SchedulingStrategy::Auto),
            last_sent: None,
        }
    }
}

impl FrameScheduler {
    pub fn new(fps: u32) -> anyhow::Result<Self> {
        anyhow::ensure!((1..=120).contains(&fps), "invalid scheduling frame rate");
        Ok(Self {
            frame_interval: Duration::from_secs_f64(1.0 / f64::from(fps)),
            qos: AdaptiveRate::new(fps, SchedulingStrategy::Auto),
            ..Self::default()
        })
    }
    pub fn configure(&mut self, enabled: bool, sequenced: bool, strategy: SchedulingStrategy) {
        self.enabled = enabled;
        self.sequenced = sequenced;
        self.strategy = strategy;
        self.reset();
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn sequenced(&self) -> bool {
        self.sequenced
    }
    pub fn strategy(&self) -> SchedulingStrategy {
        self.strategy
    }
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }
    pub fn window(&self) -> usize {
        self.window
    }
    pub fn ack_summary(&self) -> serde_json::Value {
        self.ack_samples.summary()
    }

    pub fn set_quality_scalable(&mut self, scalable: bool) {
        self.qos.set_quality_scalable(scalable);
    }
    pub fn target(&self) -> RateTarget {
        if self.enabled && self.sequenced {
            self.qos.target()
        } else {
            RateTarget {
                fps: (1.0 / self.frame_interval.as_secs_f64()).round() as u32,
                quality_percent: 100,
            }
        }
    }
    pub fn next_frame_at(&self) -> Option<Instant> {
        self.last_sent
            .map(|at| at + Duration::from_secs_f64(1.0 / f64::from(self.target().fps)))
    }
    pub fn qos_summary(&self) -> serde_json::Value {
        let mut summary = self.qos.summary();
        summary["enabled"] = json!(self.enabled && self.sequenced);
        summary["target"] = json!(self.target());
        summary
    }
    pub fn tick(&mut self, now: Instant) {
        if self.enabled
            && self.sequenced
            && self
                .pending
                .front()
                .is_some_and(|(_, at)| now.saturating_duration_since(*at) > Duration::from_secs(2))
        {
            self.qos.stall(now);
        }
    }

    pub fn waiting(&self) -> bool {
        self.enabled && self.pending.len() >= self.window
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.window = 1;
        self.dimensions = None;
        self.last_sent = None;
        self.qos.reset(self.strategy);
    }

    pub fn begin_frame(&mut self, dimensions: (u32, u32)) {
        if self.dimensions != Some(dimensions) {
            self.reset();
            self.dimensions = Some(dimensions);
        }
    }

    pub fn sent(&mut self, id: u64, now: Instant) {
        self.last_sent = Some(now);
        if self.enabled {
            self.pending.push_back((id, now));
        }
    }

    pub fn acknowledge(&mut self, id: Option<u64>, now: Instant) {
        let index = if self.sequenced {
            self.pending
                .iter()
                .position(|(pending_id, _)| Some(*pending_id) == id)
        } else {
            self.pending.front().map(|_| 0)
        };
        if let Some(index) = index {
            let elapsed = now.saturating_duration_since(self.pending[index].1);
            self.ack_samples.record(elapsed);
            if self.enabled && self.sequenced {
                self.qos.observe(self.pending[index].1, now);
            }
            self.pending.drain(..=index);
            self.window = match (self.sequenced, self.strategy) {
                (false, _) | (_, SchedulingStrategy::LowLatency) => 1,
                (true, SchedulingStrategy::Smooth) => 3,
                (true, SchedulingStrategy::Auto | SchedulingStrategy::QualityFirst) => elapsed
                    .as_nanos()
                    .div_ceil(self.frame_interval.as_nanos())
                    .clamp(1, 3)
                    as usize,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequenced_video_pipelines_only_after_first_render_and_stays_bounded() {
        let mut flow = FrameScheduler {
            enabled: true,
            sequenced: true,
            ..FrameScheduler::default()
        };
        flow.sent(1, Instant::now());
        assert!(flow.waiting());
        flow.pending[0].1 = Instant::now() - Duration::from_millis(120);
        flow.acknowledge(Some(1), Instant::now());
        assert_eq!(flow.window, 3);
        for id in 2..=4 {
            assert!(!flow.waiting());
            flow.sent(id, Instant::now());
        }
        assert!(flow.waiting());
        flow.acknowledge(Some(99), Instant::now());
        flow.acknowledge(Some(1), Instant::now());
        flow.acknowledge(None, Instant::now());
        assert_eq!(flow.pending.len(), 3);
        flow.pending[1].1 = Instant::now() - Duration::from_millis(120);
        flow.acknowledge(Some(3), Instant::now());
        assert_eq!(flow.pending.len(), 1);
        assert_eq!(flow.pending[0].0, 4);
        assert!(!flow.waiting());
        flow.reset();
        flow.sent(5, Instant::now());
        flow.acknowledge(Some(4), Instant::now());
        assert_eq!(flow.window, 1);
        assert!(flow.waiting());
        flow.acknowledge(Some(5), Instant::now());
        assert!(!flow.waiting());
    }

    #[test]
    fn diagnostic_percentiles_are_bounded_and_stale_acks_are_not_samples() {
        let mut samples = LatencySamples::default();
        assert_eq!(samples.summary()["p95_ms"], serde_json::Value::Null);
        for value in 1..=200 {
            samples.record(Duration::from_millis(value));
        }
        assert_eq!(samples.summary()["count"], 120);
        assert_eq!(samples.summary()["p50_ms"], 140.0);
        assert_eq!(samples.summary()["p95_ms"], 194.0);
        let mut flow = FrameScheduler {
            enabled: true,
            sequenced: true,
            ..FrameScheduler::default()
        };
        flow.sent(1, Instant::now());
        flow.acknowledge(Some(999), Instant::now());
        assert_eq!(flow.ack_samples.summary()["count"], 0);
        flow.acknowledge(Some(1), Instant::now());
        assert_eq!(flow.ack_samples.summary()["count"], 1);
    }

    #[test]
    fn low_delay_and_legacy_video_keep_a_single_frame_window() {
        for sequenced in [true, false] {
            let mut flow = FrameScheduler {
                enabled: true,
                sequenced,
                frame_interval: Duration::from_secs(1),
                ..FrameScheduler::default()
            };
            flow.sent(1, Instant::now());
            flow.acknowledge(Some(1), Instant::now());
            assert_eq!(flow.window, 1);
            if !sequenced {
                flow.sent(2, Instant::now());
                flow.pending[0].1 = Instant::now() - Duration::from_secs(4);
                flow.acknowledge(None, Instant::now());
                assert_eq!(flow.window, 1);
            }
        }
    }

    #[test]
    fn jpeg_resize_requires_a_new_first_frame_ack() {
        let mut flow = FrameScheduler {
            enabled: true,
            sequenced: true,
            ..FrameScheduler::default()
        };
        flow.begin_frame((1280, 720));
        flow.sent(1, Instant::now());
        flow.pending[0].1 = Instant::now() - Duration::from_millis(120);
        flow.acknowledge(Some(1), Instant::now());
        assert_eq!(flow.window, 3);
        flow.begin_frame((1280, 720));
        assert_eq!(flow.window, 3);
        flow.sent(2, Instant::now());
        flow.begin_frame((800, 600));
        flow.sent(3, Instant::now());
        flow.acknowledge(Some(2), Instant::now());
        assert_eq!(flow.window, 1);
        assert!(flow.waiting());
        flow.acknowledge(Some(3), Instant::now());
        assert!(!flow.waiting());
    }

    #[test]
    fn user_strategy_is_applied_only_after_the_first_render_ack() {
        let now = Instant::now();
        for (strategy, expected) in [
            (SchedulingStrategy::Auto, 3),
            (SchedulingStrategy::LowLatency, 1),
            (SchedulingStrategy::Smooth, 3),
        ] {
            let mut flow = FrameScheduler::new(30).unwrap();
            flow.configure(true, true, strategy);
            flow.sent(1, now);
            assert!(flow.waiting());
            flow.acknowledge(Some(1), now + Duration::from_millis(120));
            assert_eq!(flow.window(), expected);
            flow.reset();
            assert_eq!(flow.window(), 1);
        }
    }
    #[test]
    fn automatic_window_matches_deterministic_network_feedback() {
        for (delay, expected) in [(1, 1), (50, 2), (120, 3), (500, 3)] {
            let now = Instant::now();
            let mut flow = FrameScheduler::new(30).unwrap();
            flow.configure(true, true, SchedulingStrategy::Auto);
            flow.sent(1, now);
            flow.acknowledge(Some(1), now + Duration::from_millis(delay));
            assert_eq!(flow.window(), expected);
        }
        assert!(FrameScheduler::new(0).is_err());
        assert!(FrameScheduler::new(121).is_err());
    }
}
