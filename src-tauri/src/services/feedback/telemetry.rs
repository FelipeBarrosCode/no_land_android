//! Fresh, fully warmed report gauges only. Operational monitor classification is unchanged.
use super::outbox::Rental;
use crate::models::feedback::{JitterBucket, METRIC_VERSION, THRESHOLD_VERSION};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Accumulator {
    segments: HashMap<String, Segment>,
}
struct Segment {
    first_report: Instant,
    last_report: Instant,
    last_sequence: u64,
    route: String,
    route_changed: Instant,
}
impl Accumulator {
    pub fn reset(&mut self) {
        self.segments.clear();
    }
    pub fn observe(
        &mut self,
        rental: &mut Rental,
        monitor: &str,
        sequence: u64,
        measured: Instant,
        received_rtts: u64,
        value: f64,
        route: &str,
        now: Instant,
    ) -> bool {
        if !rental.confirmed
            || rental.snapshot.lifecycle.first_stream_frame_at.is_none()
            || rental.snapshot.lifecycle.destroyed_at.is_some()
            || !value.is_finite()
            || value < 0.0
            || received_rtts < 2
            || now.saturating_duration_since(measured) > Duration::from_secs(3)
        {
            return false;
        }
        // Reconnects use a new segment UUID. A long suspension restarts warmup, not totals.
        if self.segments.len() > 8 && !self.segments.contains_key(monitor) {
            self.segments.clear();
        }
        let s = self
            .segments
            .entry(monitor.into())
            .or_insert_with(|| Segment {
                first_report: measured,
                last_report: measured,
                last_sequence: 0,
                route: route.into(),
                route_changed: measured,
            });
        if sequence <= s.last_sequence {
            return false;
        }
        s.last_sequence = sequence;
        let elapsed = measured.saturating_duration_since(s.last_report);
        if elapsed > Duration::from_secs(3) {
            s.first_report = measured;
        }
        if s.route != route {
            s.route = route.into();
            s.route_changed = measured;
        }
        s.last_report = measured;
        if measured.saturating_duration_since(s.first_report) < Duration::from_secs(60) {
            return false;
        }
        // Reports sent more frequently than the nominal cadence never inflate the weight.
        if elapsed < Duration::from_millis(900) {
            return false;
        }
        let route = if measured.saturating_duration_since(s.route_changed) < Duration::from_secs(60)
        {
            "mixed"
        } else if route == "direct" || route == "relay" {
            route
        } else {
            "mixed"
        };
        let at = chrono::Utc::now().to_rfc3339();
        let buckets = &mut rental.snapshot.jitter_buckets;
        let index = buckets.iter().position(|b| {
            b.route_mode == route
                && b.metric_version == METRIC_VERSION
                && b.completeness == "full_window"
                && b.threshold_version == THRESHOLD_VERSION
                && b.nominal_report_interval_ms == 1000
        });
        let i = match index {
            Some(i) => i,
            None if buckets.len() < 8 => {
                buckets.push(JitterBucket {
                    metric_version: METRIC_VERSION.into(),
                    route_mode: route.into(),
                    completeness: "full_window".into(),
                    nominal_report_interval_ms: 1000,
                    threshold_version: THRESHOLD_VERSION.into(),
                    high_threshold_ms: 7.0,
                    severe_threshold_ms: 15.0,
                    sample_count: 0,
                    sum_ms: 0.0,
                    min_ms: value,
                    max_ms: value,
                    high_sample_count: 0,
                    severe_sample_count: 0,
                    observed_duration_ms: 0,
                    first_observed_at: at.clone(),
                    last_observed_at: at.clone(),
                });
                buckets.len() - 1
            }
            None => return false,
        };
        let b = &mut buckets[i];
        if !(b.sum_ms + value).is_finite() || b.sample_count >= 10_000_000 {
            return false;
        }
        b.sample_count += 1;
        b.sum_ms += value;
        b.min_ms = b.min_ms.min(value);
        b.max_ms = b.max_ms.max(value);
        b.high_sample_count += u64::from(value > 7.0);
        b.severe_sample_count += u64::from(value > 15.0);
        // A gauge covers at most one nominal interval; suspension/arrival gaps are not coverage.
        b.observed_duration_ms += elapsed.as_millis().min(1000) as u64;
        b.last_observed_at = at;
        rental.snapshot.revision += 1;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::super::outbox::Milestone;
    use super::*;
    use crate::models::feedback::HardwareSnapshot;
    #[test]
    fn fresh_zero_is_valid_but_stale_duplicate_and_insufficient_reports_are_not() {
        let mut a = Accumulator::default();
        let mut r = Rental::pending(1, Some("1".into()), "2".into(), HardwareSnapshot::default());
        r.confirm("3".into(), true);
        r.mark(Milestone::FirstFrame);
        let start = Instant::now();
        for n in 1..=62 {
            let t = start + Duration::from_secs(n);
            a.observe(&mut r, "monitor", n, t, 2, 0.0, "direct", t);
        }
        assert_eq!(r.snapshot.jitter_buckets[0].sample_count, 2);
        let now = start + Duration::from_secs(62);
        assert!(!a.observe(&mut r, "monitor", 62, now, 2, 0.0, "direct", now));
        assert!(!a.observe(&mut r, "monitor", 63, now, 1, 0.0, "direct", now));
        assert!(!a.observe(
            &mut r,
            "monitor",
            64,
            now,
            2,
            0.0,
            "direct",
            now + Duration::from_secs(4)
        ));
        let next = now + Duration::from_secs(1);
        assert!(a.observe(&mut r, "monitor", 65, next, 2, 8.0, "relay", next));
        assert!(r
            .snapshot
            .jitter_buckets
            .iter()
            .any(|b| b.route_mode == "mixed" && b.high_sample_count == 1));
    }
}
