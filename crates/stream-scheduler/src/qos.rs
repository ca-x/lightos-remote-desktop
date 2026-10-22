//! Connection policy over render acknowledgements, not an estimate of pure RTT.
use crate::SchedulingStrategy;
use serde::Serialize;
use serde_json::json;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const QUALITY_COOLDOWN: Duration = Duration::from_secs(2);
const FLOOR_WINDOW: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RateTarget {
    pub fps: u32,
    /// Encoder budget relative to the configured per-frame quality, not a
    /// perceptual quality measurement. Quality first always keeps this at100.
    pub quality_percent: u32,
}

#[derive(Clone)]
pub(crate) struct AdaptiveRate {
    cap: u32,
    strategy: SchedulingStrategy,
    scalable: bool,
    target: RateTarget,
    baseline: Option<Duration>,
    delays: VecDeque<Duration>,
    excess: Duration,
    last_sample: Option<Instant>,
    changed_at: Option<Instant>,
    last_quality_cut: Option<Instant>,
    last_recovery: Option<Instant>,
    last_brake: Option<Instant>,
    stall_ticks: u32,
    bad: u32,
    good: u32,
    startup: bool,
    sustained_fps: u32,
    restored_at: Option<Instant>,
    reason: &'static str,
}

impl AdaptiveRate {
    pub(crate) fn new(cap: u32, strategy: SchedulingStrategy) -> Self {
        Self {
            cap,
            strategy,
            scalable: false,
            target: RateTarget {
                fps: cap.min(15),
                quality_percent: 100,
            },
            baseline: None,
            delays: VecDeque::new(),
            excess: Duration::ZERO,
            last_sample: None,
            changed_at: None,
            last_quality_cut: None,
            last_recovery: None,
            last_brake: None,
            stall_ticks: 0,
            bad: 0,
            good: 0,
            startup: true,
            sustained_fps: cap.min(15),
            restored_at: None,
            reason: "startup",
        }
    }
    pub(crate) fn set_quality_scalable(&mut self, scalable: bool) {
        self.scalable = scalable;
        if !scalable {
            self.target.quality_percent = 100;
        }
    }
    pub(crate) fn reset(&mut self, strategy: SchedulingStrategy) {
        let scalable = self.scalable;
        *self = Self::new(self.cap, strategy);
        self.scalable = scalable;
    }
    pub(crate) fn target(&self) -> RateTarget {
        self.target
    }
    fn min_fps(&self) -> u32 {
        self.cap
            .min(if self.strategy == SchedulingStrategy::QualityFirst {
                2
            } else {
                5
            })
    }
    fn threshold(&self) -> Duration {
        Duration::from_millis(match self.strategy {
            SchedulingStrategy::LowLatency => 100,
            SchedulingStrategy::Smooth => 220,
            _ => 150,
        })
    }
    fn quality_scalable(&self) -> bool {
        self.scalable && self.strategy != SchedulingStrategy::QualityFirst
    }
    fn learn_floor(&mut self, delay: Duration) {
        self.baseline = Some(self.baseline.unwrap_or(delay).min(delay));
        if self.delays.len() == FLOOR_WINDOW {
            self.delays.pop_front();
        }
        self.delays.push_back(delay);
        if self.delays.len() == FLOOR_WINDOW {
            let low = *self.delays.iter().min().unwrap();
            let high = *self.delays.iter().max().unwrap();
            // A settled new path may replace an old minimum. A growing queue
            // cannot: its recent range must first become stable.
            if high.saturating_sub(low) <= Duration::from_millis(20) {
                let floor = self.baseline.unwrap();
                self.baseline =
                    Some(floor + low.saturating_sub(floor).min(Duration::from_millis(25)));
            }
        }
        self.excess = delay.saturating_sub(self.baseline.unwrap());
    }
    fn changed(&mut self, now: Instant, reason: &'static str) {
        self.changed_at = Some(now);
        self.reason = reason;
    }
    fn reduce_fps(&mut self, now: Instant, severe: bool) {
        self.startup = false;
        self.good = 0;
        self.bad = 0;
        let drop = if severe {
            self.target.fps.div_ceil(2)
        } else {
            self.target.fps.div_ceil(5)
        };
        self.target.fps = self.target.fps.saturating_sub(drop).max(self.min_fps());
        if self
            .restored_at
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(5))
        {
            self.sustained_fps = self.target.fps;
        }
        self.restored_at = None;
        self.changed(now, "congestion");
    }
    pub(crate) fn observe(&mut self, sent: Instant, now: Instant) {
        if self.last_brake.is_some_and(|at| sent <= at)
            || self.changed_at.is_some_and(|at| sent < at)
            || self
                .last_sample
                .is_some_and(|at| now.saturating_duration_since(at) < SAMPLE_INTERVAL)
        {
            return;
        }
        self.last_sample = Some(now);
        self.stall_ticks = 0;
        self.learn_floor(now.saturating_duration_since(sent));
        if self.excess >= self.threshold() {
            self.startup = false;
            self.good = 0;
            self.bad = self.bad.saturating_add(1);
            if self.excess >= Duration::from_secs(1) {
                self.reduce_fps(now, true);
                return;
            }
            if self.bad >= 2
                && self.quality_scalable()
                && self.target.quality_percent > 20
                && self
                    .last_quality_cut
                    .is_none_or(|at| now.saturating_duration_since(at) >= QUALITY_COOLDOWN)
            {
                self.target.quality_percent = (self.target.quality_percent * 3 / 4).max(20);
                self.last_quality_cut = Some(now);
                self.bad = 0;
                self.changed(now, "encoder_budget");
                return;
            }
            if self.bad >= 3 {
                self.reduce_fps(now, false);
            }
            return;
        }
        self.bad = 0;
        if self.excess > self.threshold() / 2 {
            self.startup = false;
            self.good = 0;
            return;
        }
        self.good = self.good.saturating_add(1);
        if self.startup && self.good >= 2 {
            self.target.fps = self.target.fps.saturating_mul(2).min(self.cap);
            self.good = 0;
            self.startup = self.target.fps < self.cap;
            self.sustained_fps = self.target.fps;
            self.changed(now, "startup");
        } else if !self.startup
            && self.good >= 2
            && self
                .last_recovery
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        {
            let before = self.target;
            self.target.quality_percent = (self.target.quality_percent + 10).min(100);
            let gradual = (self.target.fps + self.target.fps.div_ceil(10).max(1)).min(self.cap);
            self.target.fps = if self.good >= 4 && self.sustained_fps > gradual {
                self.restored_at = Some(now);
                self.sustained_fps.min(self.cap)
            } else {
                gradual
            };
            self.last_recovery = Some(now);
            if self.target != before {
                self.changed(now, "recovery");
            } else {
                self.sustained_fps = self.target.fps;
            }
        }
    }
    pub(crate) fn stall(&mut self, now: Instant) {
        if self
            .last_brake
            .is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(1))
        {
            return;
        }
        self.last_brake = Some(now);
        self.stall_ticks = self.stall_ticks.saturating_add(1);
        self.reduce_fps(now, true);
        if self.stall_ticks >= 2
            && self.quality_scalable()
            && self
                .last_quality_cut
                .is_none_or(|at| now.saturating_duration_since(at) >= QUALITY_COOLDOWN)
        {
            self.target.quality_percent = (self.target.quality_percent / 2).max(20);
            self.last_quality_cut = Some(now);
        }
        self.reason = "stall";
    }
    pub(crate) fn summary(&self) -> serde_json::Value {
        json!({"target": self.target,"baseline_ms":self.baseline.map(|v|v.as_secs_f64()*1000.0),
            "excess_ms":self.excess.as_secs_f64()*1000.0,"reason":self.reason})
    }
}
