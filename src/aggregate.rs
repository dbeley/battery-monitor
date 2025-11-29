use std::collections::BTreeSet;
use std::path::Path;

use crate::db::Sample;

fn sum_or_none(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    let mut total = 0.0;
    let mut has_value = false;
    for value in values.flatten() {
        total += value;
        has_value = true;
    }
    has_value.then_some(total)
}

fn avg_or_none(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0u64;
    for value in values.flatten() {
        total += value;
        count += 1;
    }
    if count == 0 {
        None
    } else {
        Some(total / count as f64)
    }
}

fn percent(numerator: Option<f64>, denominator: Option<f64>) -> Option<f64> {
    match (numerator, denominator) {
        (Some(num), Some(den)) if den != 0.0 => Some((num / den) * 100.0),
        _ => None,
    }
}

pub fn aggregate_group(samples: &[Sample]) -> anyhow::Result<Sample> {
    if samples.is_empty() {
        anyhow::bail!("Cannot aggregate an empty sample group");
    }
    let ts = samples[0].ts;

    let energy_now_wh = sum_or_none(samples.iter().map(|s| s.energy_now_wh));
    let energy_full_wh = sum_or_none(samples.iter().map(|s| s.energy_full_wh));
    let energy_full_design_wh = sum_or_none(samples.iter().map(|s| s.energy_full_design_wh));
    let capacity_pct = avg_or_none(samples.iter().map(|s| s.capacity_pct));

    let mut percentage = percent(energy_now_wh, energy_full_wh);
    if percentage.is_none() {
        percentage = avg_or_none(samples.iter().map(|s| s.percentage));
    }

    let mut health_pct = percent(energy_full_wh, energy_full_design_wh);
    if health_pct.is_none() {
        health_pct = avg_or_none(samples.iter().map(|s| s.health_pct));
    }

    let mut statuses: BTreeSet<String> = BTreeSet::new();
    for status in samples.iter().filter_map(|s| s.status.as_ref()) {
        statuses.insert(status.to_string());
    }
    let status = if statuses.is_empty() {
        None
    } else if statuses.len() == 1 {
        Some(statuses.into_iter().next().unwrap())
    } else {
        Some("mixed".to_string())
    };

    let mut sources: Vec<String> = samples
        .iter()
        .filter_map(|s| {
            Path::new(&s.source_path)
                .file_name()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .collect();
    sources.sort();
    sources.dedup();
    let source_path = sources.join("+");

    Ok(Sample {
        ts,
        percentage,
        capacity_pct,
        health_pct,
        energy_now_wh,
        energy_full_wh,
        energy_full_design_wh,
        status,
        source_path,
    })
}

pub fn aggregate_samples_by_timestamp(samples: &[Sample]) -> Vec<Sample> {
    if samples.is_empty() {
        return Vec::new();
    }

    let is_sorted = samples.windows(2).all(|pair| pair[0].ts <= pair[1].ts);
    if !is_sorted {
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap());
        return aggregate_samples_by_timestamp(&sorted);
    }

    aggregate_already_sorted(samples)
}

fn aggregate_already_sorted(samples: &[Sample]) -> Vec<Sample> {
    let mut aggregated = Vec::new();
    let mut start = 0usize;

    // Samples are expected to be pre-sorted by timestamp (the DB queries already order by ts).
    for idx in 1..=samples.len() {
        if idx == samples.len() || samples[idx].ts != samples[start].ts {
            if let Ok(group) = aggregate_group(&samples[start..idx]) {
                aggregated.push(group);
            }
            start = idx;
        }
    }

    aggregated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        ts: f64,
        energy_now: f64,
        energy_full: f64,
        energy_design: f64,
        capacity: Option<f64>,
        status: &str,
        source: &str,
    ) -> Sample {
        Sample {
            ts,
            percentage: None,
            capacity_pct: capacity,
            health_pct: None,
            energy_now_wh: Some(energy_now),
            energy_full_wh: Some(energy_full),
            energy_full_design_wh: Some(energy_design),
            status: Some(status.to_string()),
            source_path: source.to_string(),
        }
    }

    #[test]
    fn aggregate_group_combines_values_and_statuses() {
        let samples = vec![
            sample(1.0, 10.0, 20.0, 25.0, Some(90.0), "Discharging", "BAT0"),
            sample(1.0, 5.0, 10.0, 15.0, Some(95.0), "Charging", "BAT1"),
        ];

        let combined = aggregate_group(&samples).unwrap();

        assert_eq!(combined.energy_now_wh, Some(15.0));
        assert_eq!(combined.energy_full_wh, Some(30.0));
        assert_eq!(combined.energy_full_design_wh, Some(40.0));
        assert!((combined.percentage.unwrap_or_default() - 50.0).abs() < 1e-6);
        assert!((combined.health_pct.unwrap_or_default() - 75.0).abs() < 1e-6);
        assert_eq!(combined.capacity_pct, Some(92.5));
        assert_eq!(combined.status.as_deref(), Some("mixed"));
        assert_eq!(combined.source_path, "BAT0+BAT1");
    }

    #[test]
    fn aggregate_samples_by_timestamp_groups_events() {
        let samples = vec![
            sample(1.0, 1.0, 2.0, 3.0, None, "Discharging", "BAT0"),
            sample(2.0, 2.0, 4.0, 6.0, None, "Discharging", "BAT0"),
            sample(1.0, 0.5, 1.0, 1.5, None, "Discharging", "BAT1"),
        ];

        let aggregated = aggregate_samples_by_timestamp(&samples);

        assert_eq!(aggregated.len(), 2);
        assert_eq!(aggregated[0].ts, 1.0);
        assert_eq!(aggregated[0].energy_now_wh, Some(1.5));
        assert_eq!(aggregated[1].ts, 2.0);
        assert_eq!(aggregated[1].energy_now_wh, Some(2.0));
    }
}
