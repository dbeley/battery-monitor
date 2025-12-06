use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use log::{info, warn};
use ordered_float::OrderedFloat;
use plotters::coord::Shift;
use plotters::prelude::*;
use plotters::series::LineSeries;

use crate::aggregate::aggregate_samples_by_timestamp;
use crate::cli::ReportPreset;
use crate::db::{self, Sample};
use crate::metrics::{MetricKind, MetricSample};
use crate::timeframe::Timeframe;

pub fn load_series(db_path: &Path, timeframe: &Timeframe) -> Result<Vec<Sample>> {
    let since_ts = timeframe.since_timestamp(None);
    let raw = db::fetch_samples(db_path, since_ts)?;
    Ok(aggregate_samples_by_timestamp(&raw))
}

struct MetricSeries {
    label: String,
    points: Vec<(DateTime<Utc>, f64)>,
}

type SeriesPoints = Vec<(DateTime<Utc>, f64)>;

struct ChartSpec {
    title: String,
    y_desc: String,
    series: Vec<MetricSeries>,
}

pub fn render_plot(
    battery_samples: &[Sample],
    metrics: &[MetricSample],
    presets: &[ReportPreset],
    timeframe: &Timeframe,
    output: &Path,
) -> Result<()> {
    let charts = build_charts(battery_samples, metrics, presets, timeframe);
    if charts.is_empty() {
        warn!("No values available to plot for selected presets");
        return Ok(());
    }

    let rows = charts.len().max(1);
    let height = (rows as u32 * 260).max(260);
    let root = BitMapBackend::new(output, (1280, height)).into_drawing_area();
    root.fill(&WHITE)?;
    let areas = root.split_evenly((rows, 1));

    for (area, chart) in areas.into_iter().zip(charts.iter()) {
        plot_chart(area, chart)?;
    }

    root.present()?;
    info!("Saved plot to {}", output.display());
    Ok(())
}

fn build_charts(
    battery_samples: &[Sample],
    metrics: &[MetricSample],
    presets: &[ReportPreset],
    timeframe: &Timeframe,
) -> Vec<ChartSpec> {
    let mut charts = Vec::new();
    let label = timeframe.label.replace('_', " ");

    if presets.contains(&ReportPreset::Battery) {
        let mut series = Vec::new();
        let percent_points = battery_series(battery_samples, |s| s.percentage);
        if !percent_points.is_empty() {
            series.push(MetricSeries {
                label: "Charge %".to_string(),
                points: percent_points,
            });
        }
        let health_points = battery_series(battery_samples, |s| s.health_pct);
        if !health_points.is_empty() {
            series.push(MetricSeries {
                label: "Health %".to_string(),
                points: health_points,
            });
        }
        if !series.is_empty() {
            charts.push(ChartSpec {
                title: format!("Battery ({label})"),
                y_desc: "Percent".to_string(),
                series,
            });
        }

        let power_draw = aggregate_metric_series(metrics, MetricKind::PowerDraw, |v, _| v);
        if !power_draw.is_empty() {
            charts.push(ChartSpec {
                title: format!("Power draw ({label})"),
                y_desc: "Watts".to_string(),
                series: vec![MetricSeries {
                    label: "Discharge".to_string(),
                    points: power_draw,
                }],
            });
        }
    }

    if presets.contains(&ReportPreset::Cpu) {
        let usage = aggregate_metric_series_by_source(metrics, MetricKind::CpuUsage, |v, _| v);
        if !usage.is_empty() {
            charts.push(ChartSpec {
                title: format!("CPU usage ({label})"),
                y_desc: "Percent".to_string(),
                series: usage,
            });
        }
        let freq = aggregate_metric_series_by_source(metrics, MetricKind::CpuFrequency, |v, _| v);
        if !freq.is_empty() {
            charts.push(ChartSpec {
                title: format!("CPU frequency ({label})"),
                y_desc: "MHz".to_string(),
                series: freq,
            });
        }
    }

    if presets.contains(&ReportPreset::Gpu) {
        let usage = aggregate_metric_series_by_source(metrics, MetricKind::GpuUsage, |v, _| v);
        if !usage.is_empty() {
            charts.push(ChartSpec {
                title: format!("GPU usage ({label})"),
                y_desc: "Percent".to_string(),
                series: usage,
            });
        }
        let freq = aggregate_metric_series_by_source(metrics, MetricKind::GpuFrequency, |v, _| v);
        if !freq.is_empty() {
            charts.push(ChartSpec {
                title: format!("GPU frequency ({label})"),
                y_desc: "MHz".to_string(),
                series: freq,
            });
        }
    }

    if presets.contains(&ReportPreset::Memory) {
        let memory = aggregate_metric_series(metrics, MetricKind::MemoryUsage, |used, _| {
            bytes_to_gib(used)
        });
        if !memory.is_empty() {
            charts.push(ChartSpec {
                title: format!("Memory usage ({label})"),
                y_desc: "GiB".to_string(),
                series: vec![MetricSeries {
                    label: "Used".to_string(),
                    points: memory,
                }],
            });
        }
    }

    if presets.contains(&ReportPreset::Disk) {
        let disk =
            aggregate_metric_series(metrics, MetricKind::DiskUsage, |used, _| bytes_to_gib(used));
        if !disk.is_empty() {
            charts.push(ChartSpec {
                title: format!("Disk usage ({label})"),
                y_desc: "GiB".to_string(),
                series: vec![MetricSeries {
                    label: "Used".to_string(),
                    points: disk,
                }],
            });
        }
    }

    if presets.contains(&ReportPreset::Network) {
        let (rx, tx) = network_rate_series(metrics);
        let mut series = Vec::new();
        if !rx.is_empty() {
            series.push(MetricSeries {
                label: "Download".to_string(),
                points: rx,
            });
        }
        if !tx.is_empty() {
            series.push(MetricSeries {
                label: "Upload".to_string(),
                points: tx,
            });
        }
        if !series.is_empty() {
            charts.push(ChartSpec {
                title: format!("Network throughput ({label})"),
                y_desc: "MiB/s".to_string(),
                series,
            });
        }
    }

    if presets.contains(&ReportPreset::Temperature) {
        let temps = aggregate_metric_series_by_source(metrics, MetricKind::Temperature, |v, _| v);
        if !temps.is_empty() {
            charts.push(ChartSpec {
                title: format!("Temperature ({label})"),
                y_desc: "Celsius".to_string(),
                series: temps,
            });
        }
    }

    charts
}

fn plot_chart(area: DrawingArea<BitMapBackend, Shift>, chart: &ChartSpec) -> Result<()> {
    let mut all_points: Vec<(DateTime<Utc>, f64)> = Vec::new();
    for series in &chart.series {
        all_points.extend_from_slice(&series.points);
    }

    let Some(min_ts) = all_points.iter().map(|(ts, _)| *ts).min() else {
        return Ok(());
    };
    let Some(max_ts) = all_points.iter().map(|(ts, _)| *ts).max() else {
        return Ok(());
    };

    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (_, value) in &all_points {
        min_y = min_y.min(*value);
        max_y = max_y.max(*value);
    }
    if (max_y - min_y).abs() < 1e-6 {
        min_y -= 1.0;
        max_y += 1.0;
    }
    let padding = (max_y - min_y) * 0.05;
    let y_min = min_y - padding;
    let y_max = max_y + padding;

    let mut chart_ctx = ChartBuilder::on(&area)
        .caption(&chart.title, ("sans-serif", 20).into_font())
        .margin(12)
        .x_label_area_size(36)
        .y_label_area_size(60)
        .build_cartesian_2d(min_ts..max_ts, y_min..y_max)?;

    chart_ctx
        .configure_mesh()
        .x_labels(5)
        .y_labels(6)
        .x_desc("Time")
        .y_desc(chart.y_desc.as_str())
        .light_line_style(WHITE.mix(0.15))
        .draw()?;

    for (idx, series) in chart.series.iter().enumerate() {
        let color = Palette99::pick(idx).to_rgba();
        chart_ctx
            .draw_series(LineSeries::new(series.points.clone(), &color))?
            .label(series.label.clone())
            .legend(move |(x, y)| PathElement::new(vec![(x, y), (x + 10, y)], color));
    }

    chart_ctx
        .configure_series_labels()
        .background_style(WHITE.mix(0.8))
        .border_style(BLACK)
        .draw()?;

    Ok(())
}

fn battery_series<F>(samples: &[Sample], mut getter: F) -> Vec<(DateTime<Utc>, f64)>
where
    F: FnMut(&Sample) -> Option<f64>,
{
    samples
        .iter()
        .filter_map(|sample| {
            let ts = ts_to_datetime(sample.ts)?;
            let value = getter(sample)?;
            Some((ts, value))
        })
        .collect()
}

fn aggregate_metric_series<F>(
    metrics: &[MetricSample],
    kind: MetricKind,
    mut map_value: F,
) -> Vec<(DateTime<Utc>, f64)>
where
    F: FnMut(f64, &MetricSample) -> f64,
{
    let mut grouped: BTreeMap<OrderedFloat<f64>, Vec<f64>> = BTreeMap::new();
    for sample in metrics.iter().filter(|m| m.kind == kind) {
        if let Some(value) = sample.value {
            grouped
                .entry(OrderedFloat(sample.ts))
                .or_default()
                .push(map_value(value, sample));
        }
    }

    grouped
        .into_iter()
        .filter_map(|(ts, values)| {
            if values.is_empty() {
                return None;
            }
            let avg = values.iter().sum::<f64>() / values.len() as f64;
            ts_to_datetime(ts.into_inner()).map(|dt| (dt, avg))
        })
        .collect()
}

fn aggregate_metric_series_by_source<F>(
    metrics: &[MetricSample],
    kind: MetricKind,
    mut map_value: F,
) -> Vec<MetricSeries>
where
    F: FnMut(f64, &MetricSample) -> f64,
{
    let mut grouped: BTreeMap<String, BTreeMap<OrderedFloat<f64>, Vec<f64>>> = BTreeMap::new();
    for sample in metrics.iter().filter(|m| m.kind == kind) {
        if let Some(value) = sample.value {
            grouped
                .entry(sample.source.clone())
                .or_default()
                .entry(OrderedFloat(sample.ts))
                .or_default()
                .push(map_value(value, sample));
        }
    }

    let mut series = Vec::new();
    for (source, buckets) in grouped {
        let mut points = Vec::new();
        for (ts, values) in buckets {
            if values.is_empty() {
                continue;
            }
            let avg = values.iter().sum::<f64>() / values.len() as f64;
            if let Some(dt) = ts_to_datetime(ts.into_inner()) {
                points.push((dt, avg));
            }
        }
        if !points.is_empty() {
            series.push(MetricSeries {
                label: source,
                points,
            });
        }
    }
    series
}

fn network_rate_series(metrics: &[MetricSample]) -> (SeriesPoints, SeriesPoints) {
    let mut by_iface: BTreeMap<&str, Vec<&MetricSample>> = BTreeMap::new();
    for sample in metrics
        .iter()
        .filter(|s| s.kind == MetricKind::NetworkBytes)
    {
        by_iface.entry(&sample.source).or_default().push(sample);
    }

    let mut rx_series = Vec::new();
    let mut tx_series = Vec::new();
    for (_iface, mut samples) in by_iface {
        samples.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap());
        for window in samples.windows(2) {
            let prev = window[0];
            let next = window[1];
            let dt = next.ts - prev.ts;
            if dt <= 0.0 {
                continue;
            }
            let rx_rate = rate_from_counters(
                detail_number(prev, "rx_bytes"),
                detail_number(next, "rx_bytes"),
                dt,
            );
            let tx_rate = rate_from_counters(
                detail_number(prev, "tx_bytes"),
                detail_number(next, "tx_bytes"),
                dt,
            );
            let ts = match ts_to_datetime(next.ts) {
                Some(v) => v,
                None => continue,
            };
            if let Some(rx) = rx_rate {
                rx_series.push((ts, rx / 1_048_576.0));
            }
            if let Some(tx) = tx_rate {
                tx_series.push((ts, tx / 1_048_576.0));
            }
        }
    }
    rx_series.sort_by_key(|(ts, _)| *ts);
    tx_series.sort_by_key(|(ts, _)| *ts);
    (rx_series, tx_series)
}

fn rate_from_counters(previous: Option<f64>, current: Option<f64>, dt: f64) -> Option<f64> {
    match (previous, current) {
        (Some(prev), Some(next)) if next >= prev && dt > 0.0 => Some((next - prev) / dt),
        _ => None,
    }
}

fn detail_number(sample: &MetricSample, key: &str) -> Option<f64> {
    sample
        .details
        .get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn bytes_to_gib(used: f64) -> f64 {
    used / (1024.0 * 1024.0 * 1024.0)
}

fn ts_to_datetime(ts: f64) -> Option<DateTime<Utc>> {
    let seconds = ts.trunc() as i64;
    let nanos = ((ts.fract() * 1_000_000_000.0).round() as u32).min(999_999_999);
    Utc.timestamp_opt(seconds, nanos).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric_sample(source: &str, ts: f64, value: f64, kind: MetricKind) -> MetricSample {
        MetricSample {
            ts,
            kind,
            source: source.to_string(),
            value: Some(value),
            unit: None,
            details: serde_json::Value::Null,
        }
    }

    #[test]
    fn aggregate_metric_series_is_per_source() {
        let metrics = vec![
            metric_sample("cpu0", 0.0, 10.0, MetricKind::CpuUsage),
            metric_sample("cpu1", 0.0, 20.0, MetricKind::CpuUsage),
            metric_sample("cpu0", 60.0, 30.0, MetricKind::CpuUsage),
        ];

        let series = aggregate_metric_series_by_source(&metrics, MetricKind::CpuUsage, |v, _| v);
        assert_eq!(series.len(), 2);
        let cpu0 = series.iter().find(|s| s.label == "cpu0").unwrap();
        let cpu1 = series.iter().find(|s| s.label == "cpu1").unwrap();
        assert_eq!(cpu0.points.len(), 2);
        assert_eq!(cpu1.points.len(), 1);
    }
}
