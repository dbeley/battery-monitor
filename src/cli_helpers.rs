use std::borrow::Cow;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};

use crate::db::Sample;
use crate::timeframe::Timeframe;

fn sanitize_component(value: &str) -> Cow<'_, str> {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Cow::Borrowed(value);
    }
    let replaced: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    Cow::Owned(replaced)
}

pub fn default_graph_path(
    timeframe: &str,
    base_dir: Option<&Path>,
    now: Option<DateTime<Local>>,
) -> PathBuf {
    let current = now.unwrap_or_else(Local::now);
    let tz_label = current.format("%Z").to_string();
    let tz_name = sanitize_component(&tz_label);
    let timeframe_label = timeframe.replace('-', "_");
    let timestamp = current.format("%Y-%m-%d_%H-%M-%S");
    let filename = format!(
        "battery_monitor_{}_{}_{}.png",
        timeframe_label, timestamp, tz_name
    );
    base_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(filename)
}

pub fn bucket_span_seconds(timeframe: &Timeframe) -> i64 {
    match timeframe.seconds {
        None => 7 * 24 * 3600,
        Some(window) if window <= 6.0 * 3600.0 => 20 * 60,
        Some(window) if window <= 24.0 * 3600.0 => 3600,
        Some(window) if window <= 3.0 * 24.0 * 3600.0 => 2 * 3600,
        Some(window) if window <= 7.0 * 24.0 * 3600.0 => 6 * 3600,
        Some(window) if window <= 30.0 * 24.0 * 3600.0 => 24 * 3600,
        Some(window) if window <= 90.0 * 24.0 * 3600.0 => 3 * 24 * 3600,
        _ => 7 * 24 * 3600,
    }
}

pub fn bucket_start(ts: f64, bucket_seconds: i64) -> DateTime<Local> {
    let local_dt = Local.timestamp_opt(ts as i64, 0).unwrap();
    let offset_seconds = -local_dt.offset().utc_minus_local(); // convert to python-style offset
    let bucket_epoch = (((ts + offset_seconds as f64) / bucket_seconds as f64).floor()
        * bucket_seconds as f64)
        - offset_seconds as f64;
    let aligned = bucket_epoch.max(0.0) as i64;
    Local.timestamp_opt(aligned, 0).unwrap()
}

#[derive(Debug, Default, PartialEq)]
pub struct AverageRates {
    pub discharge_w: Option<f64>,
    pub charge_w: Option<f64>,
}

#[derive(Default)]
struct RateAccumulator {
    delta: f64,
    hours: f64,
}

impl RateAccumulator {
    fn record(&mut self, delta_wh: f64, dt_hours: f64) {
        self.delta += delta_wh;
        self.hours += dt_hours;
    }

    fn average(&self) -> Option<f64> {
        if self.hours == 0.0 || self.delta == 0.0 {
            None
        } else {
            Some(self.delta / self.hours)
        }
    }
}

pub fn average_rates<'a>(samples: impl IntoIterator<Item = &'a Sample>) -> AverageRates {
    const MAX_GAP_HOURS: f64 = 5.0 / 60.0;

    let mut discharge = RateAccumulator::default();
    let mut charge = RateAccumulator::default();
    let mut iter = samples.into_iter().filter(|s| s.energy_now_wh.is_some());
    let mut previous = match iter.next() {
        Some(sample) => sample,
        None => return AverageRates::default(),
    };

    for current in iter {
        if current.ts < previous.ts {
            previous = current;
            continue;
        }
        let dt_hours = (current.ts - previous.ts) / 3600.0;
        if dt_hours > 0.0 && dt_hours <= MAX_GAP_HOURS {
            let delta = current.energy_now_wh.unwrap() - previous.energy_now_wh.unwrap();
            if delta > 0.0 && is_charging(previous) && is_charging(current) {
                charge.record(delta, dt_hours);
            } else if delta < 0.0 && is_discharging(previous) && is_discharging(current) {
                discharge.record(-delta, dt_hours);
            }
        }
        previous = current;
    }

    AverageRates {
        discharge_w: discharge.average(),
        charge_w: charge.average(),
    }
}

fn is_discharging(sample: &Sample) -> bool {
    sample
        .status
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("discharging"))
        .unwrap_or(true)
}

fn is_charging(sample: &Sample) -> bool {
    sample
        .status
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("charging"))
        .unwrap_or(false)
}

pub fn average_discharge_w(samples: &[Sample]) -> Option<f64> {
    average_rates(samples).discharge_w
}

pub fn average_charge_w(samples: &[Sample]) -> Option<f64> {
    average_rates(samples).charge_w
}

pub fn estimate_runtime_hours(
    avg_discharge_w: Option<f64>,
    current_sample: &Sample,
) -> Option<f64> {
    let avg = avg_discharge_w?;
    if avg <= 0.0 {
        return None;
    }
    let capacity_wh = current_sample
        .energy_full_wh
        .or(current_sample.energy_full_design_wh)?;
    if capacity_wh <= 0.0 {
        return None;
    }
    Some(capacity_wh / avg)
}

pub fn format_runtime(hours: Option<f64>) -> String {
    match hours {
        None => "--".to_string(),
        Some(value) if value.is_sign_negative() || !value.is_finite() => "--".to_string(),
        Some(value) => {
            let minutes = (value * 60.0).floor() as i64;
            let hrs = minutes / 60;
            let mins = minutes % 60;
            format!("{hrs}h{mins:02}m")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn sample(
        ts: f64,
        energy_now: f64,
        energy_full: Option<f64>,
        energy_full_design: Option<f64>,
        status: Option<&str>,
    ) -> Sample {
        Sample {
            ts,
            percentage: None,
            capacity_pct: None,
            health_pct: None,
            energy_now_wh: Some(energy_now),
            energy_full_wh: energy_full,
            energy_full_design_wh: energy_full_design,
            status: status.map(|s| s.to_string()),
            source_path: "/dev/null".to_string(),
        }
    }

    #[test]
    fn default_graph_path_has_timeframe_and_timestamp() {
        let now = Local.with_ymd_and_hms(2025, 11, 28, 1, 30, 42).unwrap();
        let path = default_graph_path("last_3_hours", Some(Path::new("/tmp")), Some(now));
        let tz_label = now.format("%Z").to_string();
        let tz = sanitize_component(&tz_label);
        let expected = PathBuf::from(format!(
            "/tmp/battery_monitor_last_3_hours_2025-11-28_01-30-42_{}.png",
            tz
        ));
        assert_eq!(path, expected);
    }

    #[test]
    fn average_discharge_and_runtime_estimates() {
        let samples = vec![
            sample(0.0, 60.0, Some(60.0), Some(70.0), None),
            sample(300.0, 59.6, Some(60.0), Some(70.0), None),
            sample(600.0, 59.2, Some(60.0), Some(70.0), None),
        ];
        let avg = average_discharge_w(&samples).unwrap();
        let runtime_hours = estimate_runtime_hours(Some(avg), samples.last().unwrap()).unwrap();
        assert!((avg - 4.8).abs() < 0.01);
        assert!((runtime_hours - 12.5).abs() < 0.01);
        assert_eq!(format_runtime(Some(runtime_hours)), "12h30m");

        let design_sample = sample(3600.0, 55.0, None, Some(80.0), None);
        let design_runtime = estimate_runtime_hours(Some(avg), &design_sample).unwrap();
        assert!((design_runtime - 16.67).abs() < 0.02);
    }

    #[test]
    fn average_discharge_ignores_large_gaps() {
        let samples = vec![
            sample(0.0, 60.0, Some(60.0), Some(70.0), None),
            sample(300.0, 59.5, Some(60.0), Some(70.0), None),
            sample(1800.0, 59.4, Some(60.0), Some(70.0), None),
        ];
        let avg = average_discharge_w(&samples).unwrap();
        assert!((avg - 6.0).abs() < 0.01);

        let runtime_hours = estimate_runtime_hours(Some(avg), samples.last().unwrap()).unwrap();
        assert!((runtime_hours - 10.0).abs() < 0.01);
    }

    #[test]
    fn average_discharge_ignores_charging_segments() {
        let samples = vec![
            sample(0.0, 60.0, Some(60.0), Some(70.0), Some("Discharging")),
            sample(300.0, 59.0, Some(60.0), Some(70.0), Some("Discharging")),
            sample(600.0, 60.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(900.0, 59.5, Some(60.0), Some(70.0), Some("Discharging")),
            sample(1200.0, 59.0, Some(60.0), Some(70.0), Some("Discharging")),
        ];
        let avg = average_discharge_w(&samples).unwrap();
        assert!((avg - 9.0).abs() < 0.01);
    }

    #[test]
    fn average_charge_tracks_charging_only() {
        let samples = vec![
            sample(0.0, 50.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(300.0, 52.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(600.0, 52.5, Some(60.0), Some(70.0), Some("Charging")),
            sample(900.0, 52.2, Some(60.0), Some(70.0), Some("Discharging")),
            sample(1200.0, 53.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(1500.0, 54.5, Some(60.0), Some(70.0), Some("Charging")),
        ];
        let avg = average_charge_w(&samples).unwrap();
        assert!((avg - 16.0).abs() < 0.01);
    }

    #[test]
    fn average_rates_compute_charge_and_discharge_together() {
        let samples = vec![
            sample(0.0, 50.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(300.0, 51.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(600.0, 52.0, Some(60.0), Some(70.0), Some("Charging")),
            sample(900.0, 51.5, Some(60.0), Some(70.0), Some("Discharging")),
            sample(1200.0, 51.0, Some(60.0), Some(70.0), Some("Discharging")),
        ];

        let rates = average_rates(&samples);
        assert!((rates.charge_w.unwrap() - 12.0).abs() < 0.01);
        assert!((rates.discharge_w.unwrap() - 6.0).abs() < 0.01);
    }

    #[test]
    fn bucket_alignment_matches_expected_windows() {
        use crate::timeframe::build_timeframe;
        let timeframe = build_timeframe(6, 0, 0, false).unwrap();
        let span = bucket_span_seconds(&timeframe);
        let sample_dt = Local::now()
            .with_minute(37)
            .unwrap()
            .with_second(42)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();
        let bucket = bucket_start(sample_dt.timestamp() as f64, span);

        assert_eq!(span, 20 * 60);
        assert_eq!(bucket.minute() % 20, 0);
        assert_eq!(bucket.second(), 0);

        let one_day = build_timeframe(0, 1, 0, false).unwrap();
        let span_day = bucket_span_seconds(&one_day);
        let bucket_day = bucket_start(sample_dt.timestamp() as f64, span_day);
        assert_eq!(span_day, 3600);
        assert_eq!(bucket_day.hour(), sample_dt.hour());
        assert_eq!(bucket_day.minute(), 0);
        assert_eq!(bucket_day.second(), 0);
    }
}
