use std::time::{Duration, Instant};
use stream_scheduler::{FrameScheduler, SchedulingStrategy};

fn flow(strategy: SchedulingStrategy, fps: u32) -> FrameScheduler {
    let mut flow = FrameScheduler::new(fps).unwrap();
    flow.configure(true, true, strategy);
    flow.set_quality_scalable(true);
    flow
}
fn sample(flow: &mut FrameScheduler, id: u64, clock: &mut Instant, delay_ms: u64) {
    flow.sent(id, *clock);
    flow.acknowledge(Some(id), *clock + Duration::from_millis(delay_ms));
    *clock += Duration::from_millis(delay_ms + 500);
}
fn warm(flow: &mut FrameScheduler, clock: &mut Instant) {
    for id in 1..=5 {
        sample(flow, id, clock, 20);
    }
}

#[test]
fn constant_high_delay_is_a_baseline_not_congestion() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    for id in 1..=40 {
        sample(&mut flow, id, &mut clock, 500);
    }
    assert_eq!(flow.target().fps, 30);
    assert_eq!(flow.target().quality_percent, 100);
    assert_eq!(flow.qos_summary()["baseline_ms"], 500.0);
}
#[test]
fn one_spike_and_invalid_or_duplicate_acks_cannot_cut_rate() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    let target = flow.target();
    sample(&mut flow, 6, &mut clock, 1200);
    assert_eq!(flow.target(), target);
    for _ in 0..10 {
        flow.acknowledge(Some(6), clock);
        flow.acknowledge(Some(999), clock);
    }
    assert_eq!(flow.target(), target);
    sample(&mut flow, 7, &mut clock, 20);
    assert_eq!(flow.target(), target);
}
#[test]
fn auto_cuts_encoder_budget_before_ordinary_fps_reduction() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    sample(&mut flow, 6, &mut clock, 400);
    assert_eq!(flow.target().quality_percent, 100);
    sample(&mut flow, 7, &mut clock, 400);
    assert!(flow.target().quality_percent < 100);
    assert_eq!(flow.target().fps, 30);
    for id in 8..=30 {
        sample(&mut flow, id, &mut clock, 400);
    }
    assert!(flow.target().fps < 30);
    assert!(flow.target().fps >= 5);
    assert!(flow.target().quality_percent >= 20);
}
#[test]
fn quality_first_preserves_budget_and_reduces_fps() {
    let mut flow = flow(SchedulingStrategy::QualityFirst, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    for id in 6..=30 {
        sample(&mut flow, id, &mut clock, 400);
    }
    assert_eq!(flow.target().quality_percent, 100);
    assert!((2..30).contains(&flow.target().fps));
}
#[test]
fn fresh_samples_are_time_spaced_and_do_not_repenalize_an_old_epoch() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    for id in 6..=8 {
        flow.sent(id, clock);
    }
    flow.acknowledge(Some(6), clock + Duration::from_millis(400));
    flow.acknowledge(Some(7), clock + Duration::from_millis(401));
    flow.acknowledge(Some(8), clock + Duration::from_millis(402));
    assert_eq!(flow.target().quality_percent, 100);
    clock += Duration::from_secs(1);
    sample(&mut flow, 9, &mut clock, 400);
    let target = flow.target();
    assert!(target.quality_percent < 100);
    flow.acknowledge(Some(8), clock + Duration::from_secs(10));
    assert_eq!(flow.target(), target);
}
#[test]
fn rising_queue_does_not_raise_the_baseline() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    for id in 6..=45 {
        sample(&mut flow, id, &mut clock, 40 + (id - 6) * 20);
    }
    assert_eq!(flow.qos_summary()["baseline_ms"], 20.0);
    assert!(flow.target().fps < 30);
}
#[test]
fn settled_path_floor_can_eventually_be_relearned() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    for id in 6..=80 {
        sample(&mut flow, id, &mut clock, 250);
    }
    assert_eq!(flow.qos_summary()["baseline_ms"], 250.0);
    assert_eq!(flow.target().fps, 30);
    assert_eq!(flow.target().quality_percent, 100);
}
#[test]
fn idle_is_not_a_stall_and_a_late_ack_is_not_a_second_brake() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    flow.tick(clock + Duration::from_secs(30));
    assert_eq!(flow.target().fps, 30);
    flow.sent(6, clock);
    flow.tick(clock + Duration::from_secs(3));
    assert_eq!(flow.target().fps, 15);
    flow.tick(clock + Duration::from_millis(3100));
    assert_eq!(flow.target().fps, 15);
    flow.tick(clock + Duration::from_secs(4));
    let braked = flow.target();
    assert!(braked.fps < 15);
    flow.acknowledge(Some(6), clock + Duration::from_millis(4100));
    assert_eq!(flow.target(), braked);
}
#[test]
fn recovery_returns_towards_caps_and_resize_discards_old_feedback() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    flow.begin_frame((1280, 720));
    warm(&mut flow, &mut clock);
    for id in 6..=18 {
        sample(&mut flow, id, &mut clock, 400);
    }
    assert!(flow.target().quality_percent < 100);
    for id in 19..=60 {
        sample(&mut flow, id, &mut clock, 20);
    }
    assert_eq!(flow.target().fps, 30);
    assert_eq!(flow.target().quality_percent, 100);
    flow.sent(61, clock);
    flow.begin_frame((800, 600));
    let reset = flow.target();
    assert_eq!(reset.fps, 15);
    flow.acknowledge(Some(61), clock + Duration::from_secs(8));
    assert_eq!(flow.target(), reset);
    assert_eq!(flow.window(), 1);
}
#[test]
fn fps_caps_and_legacy_clients_remain_bounded() {
    for cap in [1, 2, 10, 60, 120] {
        let mut flow = flow(SchedulingStrategy::Auto, cap);
        let mut clock = Instant::now();
        for id in 1..=10 {
            sample(&mut flow, id, &mut clock, 20);
            assert!(flow.target().fps <= cap);
        }
        assert_eq!(flow.target().fps, cap);
        flow.configure(true, false, SchedulingStrategy::Auto);
        assert_eq!(flow.target().fps, cap);
        assert_eq!(flow.target().quality_percent, 100);
        flow.configure(false, true, SchedulingStrategy::Auto);
        flow.tick(clock + Duration::from_secs(30));
        assert_eq!(flow.target().fps, cap);
    }
}
#[test]
fn pacing_uses_target_fps_and_resets_on_new_dimensions() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let now = Instant::now();
    assert!(flow.next_frame_at().is_none());
    flow.sent(1, now);
    let interval = flow.next_frame_at().unwrap().duration_since(now);
    assert!(interval >= Duration::from_millis(60));
    flow.begin_frame((800, 600));
    assert!(flow.next_frame_at().is_none());
}
#[test]
fn encoders_without_scalable_quality_still_control_fps() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    flow.set_quality_scalable(false);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    for id in 6..=20 {
        sample(&mut flow, id, &mut clock, 400);
    }
    assert_eq!(flow.target().quality_percent, 100);
    assert!(flow.target().fps < 30);
}

#[test]
fn native_skips_are_paced_from_the_encode_attempt_without_consuming_credit() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let now = Instant::now();
    flow.frame_started(now);
    assert_eq!(flow.in_flight(), 0);
    assert!(flow.next_frame_at().unwrap() >= now + Duration::from_millis(60));
    flow.frame_started(now + Duration::from_secs(1));
    assert!(flow.next_frame_at().unwrap() >= now + Duration::from_millis(1060));
}
#[test]
fn strategies_have_different_queue_tolerance_and_quality_first_survives_stalls() {
    for (strategy, reduced) in [
        (SchedulingStrategy::LowLatency, true),
        (SchedulingStrategy::Auto, true),
        (SchedulingStrategy::Smooth, false),
    ] {
        let mut flow = flow(strategy, 30);
        let mut clock = Instant::now();
        warm(&mut flow, &mut clock);
        for id in 6..=9 {
            sample(&mut flow, id, &mut clock, 200);
        }
        assert_eq!(flow.target().quality_percent < 100, reduced);
    }
    let mut flow = flow(SchedulingStrategy::QualityFirst, 30);
    let now = Instant::now();
    flow.sent(1, now);
    for secs in 3..=10 {
        flow.tick(now + Duration::from_secs(secs));
    }
    assert_eq!(flow.target().quality_percent, 100);
    assert_eq!(flow.target().fps, 2);
}

#[test]
fn slowly_rising_queue_is_not_learned_as_a_new_path_floor() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    for step in 1..=1000 {
        sample(&mut flow, 5 + step, &mut clock, 20 + step);
    }
    assert_eq!(flow.qos_summary()["baseline_ms"], 20.0);
    assert!(flow.target().fps < 30);
    assert!(flow.target().quality_percent < 100);
}

#[test]
fn confirmed_severe_delay_brakes_fps_without_waiting_for_a_stall() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let mut clock = Instant::now();
    warm(&mut flow, &mut clock);
    sample(&mut flow, 6, &mut clock, 1200);
    assert_eq!(flow.target().fps, 30);
    sample(&mut flow, 7, &mut clock, 1200);
    assert_eq!(flow.target().fps, 15);
}

#[test]
fn strategy_changes_and_native_skips_request_one_paced_refresh() {
    let mut flow = flow(SchedulingStrategy::Auto, 30);
    let now = Instant::now();
    flow.begin_frame((800, 600));
    flow.frame_started(now);
    assert!(!flow.refresh_requested());
    flow.frame_skipped();
    assert!(flow.refresh_requested());
    assert_eq!(flow.in_flight(), 0);
    assert!(flow.next_frame_at().unwrap() >= now + Duration::from_millis(60));
    flow.frame_started(now + Duration::from_secs(1));
    assert!(!flow.refresh_requested());
    flow.sent(1, now + Duration::from_secs(1));
    flow.acknowledge(Some(1), now + Duration::from_millis(1020));
    flow.tick(now + Duration::from_secs(20));
    assert!(
        !flow.refresh_requested(),
        "ACKs and idle ticks must not refresh static frames"
    );
    flow.configure(true, true, SchedulingStrategy::QualityFirst);
    assert!(
        flow.refresh_requested(),
        "a new strategy must update a static image"
    );
    flow.frame_started(now + Duration::from_secs(21));
    assert!(!flow.refresh_requested());
}
