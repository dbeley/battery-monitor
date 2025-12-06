use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};

use chrono::{DateTime, Local};

use crate::aggregate::aggregate_samples_by_timestamp;
use crate::cli_helpers::{
    average_rates, bucket_span_seconds, bucket_start, default_graph_path, estimate_runtime_hours,
    format_runtime,
};
use crate::collector::{collect_loop, collect_once, resolve_db_path};
use crate::db::{self, Sample};
use crate::graph;
use crate::metrics::{MetricKind, MetricSample};
use crate::timeframe::{build_timeframe, Timeframe};

#[derive(Parser)]
#[command(name = "symmetri", version)]
#[command(
    about = "System metrics collection for Linux/NixOS (battery, CPU, GPU, network, RAM, disk, thermals)"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum ReportPreset {
    Battery,
    Cpu,
    Gpu,
    Memory,
    Network,
    Temperature,
    Disk,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Collect system metrics once (or repeatedly with --interval)
    Collect {
        /// Path to SQLite database (or set SYMMETRI_DB)
        #[arg(long = "db")]
        db_path: Option<PathBuf>,
        /// Optional interval seconds to loop forever
        #[arg(long = "interval")]
        interval: Option<u64>,
        /// Enable debug logging
        #[arg(short, long)]
        verbose: bool,
    },
    /// Render a timeframe report (optionally save a graph image)
    Report {
        /// Window in hours (used when days/months are zero)
        #[arg(long = "hours", default_value_t = 6)]
        hours: u64,
        /// Window in days (overrides hours when non-zero)
        #[arg(long = "days", default_value_t = 0)]
        days: u64,
        /// Window in months (~30d each; overrides days/hours when non-zero)
        #[arg(long = "months", default_value_t = 0)]
        months: u64,
        /// Ignore timeframe limits and use the entire history
        #[arg(long = "all")]
        all_time: bool,
        /// Path to SQLite database (or set BATTERY_MONITOR_DB)
        #[arg(long = "db")]
        db_path: Option<PathBuf>,
        /// Save a graph image with an auto-generated name
        #[arg(long = "graph", short = 'g')]
        graph: bool,
        /// Custom path for the graph image (png/pdf/etc); overrides --graph name
        #[arg(long = "graph-path")]
        graph_path: Option<PathBuf>,
        /// Limit metrics to specific sensor names (repeatable)
        #[arg(long = "sensor", value_name = "NAME", num_args = 0..)]
        sensor_filters: Vec<String>,
        /// Which report presets to render (repeatable)
        #[arg(
            long = "preset",
            value_enum,
            num_args = 0..,
            default_values_t = [ReportPreset::Battery]
        )]
        presets: Vec<ReportPreset>,
        /// Enable debug logging
        #[arg(short, long)]
        verbose: bool,
    },
}

fn configure_logging(verbose: bool) {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default());
    builder.format(|buf, record| writeln!(buf, "{}", record.args()));
    if verbose {
        builder.filter_level(log::LevelFilter::Debug);
    } else {
        builder.filter_level(log::LevelFilter::Info);
    }
    let _ = builder.try_init();
}

fn normalize_presets(mut presets: Vec<ReportPreset>) -> Vec<ReportPreset> {
    if presets.is_empty() {
        return vec![ReportPreset::Battery];
    }
    presets.sort();
    presets.dedup();
    presets
}

fn metric_kinds_for_presets(presets: &[ReportPreset]) -> Vec<MetricKind> {
    let mut kinds = Vec::new();
    for preset in presets {
        match preset {
            ReportPreset::Battery => kinds.push(MetricKind::PowerDraw),
            ReportPreset::Cpu => {
                kinds.push(MetricKind::CpuUsage);
                kinds.push(MetricKind::CpuFrequency);
            }
            ReportPreset::Gpu => {
                kinds.push(MetricKind::GpuUsage);
                kinds.push(MetricKind::GpuFrequency);
            }
            ReportPreset::Memory => kinds.push(MetricKind::MemoryUsage),
            ReportPreset::Network => kinds.push(MetricKind::NetworkBytes),
            ReportPreset::Temperature => kinds.push(MetricKind::Temperature),
            ReportPreset::Disk => kinds.push(MetricKind::DiskUsage),
        }
    }
    kinds.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    kinds.dedup();
    kinds
}

fn has_data_for_preset(preset: ReportPreset, samples: &[Sample], metrics: &[MetricSample]) -> bool {
    match preset {
        ReportPreset::Battery => {
            !samples.is_empty() || metrics.iter().any(|m| m.kind == MetricKind::PowerDraw)
        }
        ReportPreset::Cpu => metrics
            .iter()
            .any(|m| matches!(m.kind, MetricKind::CpuUsage | MetricKind::CpuFrequency)),
        ReportPreset::Gpu => metrics
            .iter()
            .any(|m| matches!(m.kind, MetricKind::GpuUsage | MetricKind::GpuFrequency)),
        ReportPreset::Memory => metrics.iter().any(|m| m.kind == MetricKind::MemoryUsage),
        ReportPreset::Network => metrics.iter().any(|m| m.kind == MetricKind::NetworkBytes),
        ReportPreset::Temperature => metrics.iter().any(|m| m.kind == MetricKind::Temperature),
        ReportPreset::Disk => metrics.iter().any(|m| m.kind == MetricKind::DiskUsage),
    }
}

pub fn run<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    match cli.command {
        Commands::Collect {
            db_path,
            interval,
            verbose,
        } => {
            configure_logging(verbose);
            if let Some(interval) = interval {
                collect_loop(interval, db_path.as_deref(), None)?;
            } else {
                let code = collect_once(db_path.as_deref(), None)?;
                if code != 0 {
                    std::process::exit(code);
                }
            }
        }
        Commands::Report {
            hours,
            days,
            months,
            all_time,
            db_path,
            graph: graph_flag,
            graph_path,
            presets,
            sensor_filters,
            verbose,
        } => {
            configure_logging(verbose);
            let timeframe = build_timeframe(hours as i64, days as i64, months as i64, all_time)?;
            let resolved = resolve_db_path(db_path.as_deref());
            let presets = normalize_presets(presets);
            let metric_kinds = metric_kinds_for_presets(&presets);

            let battery_total = db::count_samples(&resolved, None)?;
            let metric_total = db::count_metric_samples(&resolved, None)?;
            if battery_total == 0 && metric_total == 0 {
                println!("No records available; collect data first.");
                std::process::exit(1);
            }

            let since_ts = timeframe.since_timestamp(None);
            let raw_samples =
                if presets.contains(&ReportPreset::Battery) || graph_flag || graph_path.is_some() {
                    db::fetch_samples(&resolved, since_ts)?
                } else {
                    Vec::new()
                };
            let metric_samples =
                db::fetch_metric_samples(&resolved, since_ts, Some(&metric_kinds))?;
            let metric_samples = filter_metrics_by_source(&metric_samples, &sensor_filters);
            let timeframe_record_count = raw_samples.len();
            let samples = aggregate_samples_by_timestamp(&raw_samples);
            let has_selected_data = presets
                .iter()
                .any(|preset| has_data_for_preset(*preset, &samples, &metric_samples));
            if !has_selected_data {
                println!(
                    "No records for the selected presets in {}; try a broader timeframe or enable those collectors.",
                    timeframe.label.replace('_', " ")
                );
                std::process::exit(1);
            }

            let output_path = match (graph_path, graph_flag) {
                (Some(path), _) => Some(path),
                (None, true) => Some(default_graph_path(
                    &timeframe.label,
                    None,
                    Some(Local::now()),
                )),
                _ => None,
            };

            if let Some(path) = output_path {
                if samples.is_empty() && metric_samples.is_empty() {
                    println!("Skipping graph output; no data in timeframe.");
                } else {
                    let battery_for_plot: &[Sample] = if presets.contains(&ReportPreset::Battery) {
                        &samples
                    } else {
                        &[]
                    };
                    graph::render_plot(
                        battery_for_plot,
                        &metric_samples,
                        &presets,
                        &timeframe,
                        &path,
                    )?;
                }
            }

            summarize(
                &samples,
                &timeframe,
                timeframe_record_count,
                &metric_samples,
                &presets,
            );
        }
    }
    Ok(())
}

fn summarize(
    timeframe_samples: &[Sample],
    timeframe: &Timeframe,
    timeframe_records: usize,
    metrics: &[MetricSample],
    presets: &[ReportPreset],
) {
    let timeframe_label = timeframe.label.replace('_', " ");
    let bucket_seconds =
        bucket_span_seconds(timeframe, data_span_seconds(timeframe_samples, metrics));
    let battery_rates = average_rates(timeframe_samples);
    let power_draw_stats = average_for_kind(metrics, MetricKind::PowerDraw);
    let avg_discharge_w = power_draw_stats.average().or(battery_rates.discharge_w);
    let est_runtime_hours = timeframe_samples
        .last()
        .and_then(|sample| estimate_runtime_hours(avg_discharge_w, sample));
    let power_draw_by_bucket =
        bucket_stats_for_kind(metrics, MetricKind::PowerDraw, bucket_seconds);
    let network_rates = compute_network_rates(metrics);

    if presets.contains(&ReportPreset::Battery) {
        println!(
            "\nBattery summary ({})\n{}",
            timeframe_label,
            battery_summary_table(
                timeframe_records,
                avg_discharge_w,
                battery_rates.charge_w,
                est_runtime_hours
            )
        );

        if timeframe_samples.is_empty() {
            println!("\nNo battery samples available for buckets in {timeframe_label}.");
        } else {
            println!(
                "\nBattery stats ({})\n{}",
                timeframe.label.replace('_', " "),
                battery_stats_table(timeframe_samples, &power_draw_by_bucket, bucket_seconds)
            );
        }
    }

    if presets.contains(&ReportPreset::Cpu) {
        let usage_buckets =
            bucket_stats_for_kind_by_source(metrics, MetricKind::CpuUsage, bucket_seconds);
        let freq_buckets =
            bucket_stats_for_kind_by_source(metrics, MetricKind::CpuFrequency, bucket_seconds);
        if usage_buckets.is_empty() && freq_buckets.is_empty() {
            println!("\nNo CPU samples available for {timeframe_label}.");
        } else {
            println!(
                "\nCPU stats ({})\n{}",
                timeframe.label.replace('_', " "),
                cpu_stats_table(bucket_seconds, &usage_buckets, &freq_buckets)
            );
        }
    }

    if presets.contains(&ReportPreset::Gpu) {
        let usage_buckets =
            bucket_stats_for_kind_by_source(metrics, MetricKind::GpuUsage, bucket_seconds);
        let freq_buckets =
            bucket_stats_for_kind_by_source(metrics, MetricKind::GpuFrequency, bucket_seconds);
        if usage_buckets.is_empty() && freq_buckets.is_empty() {
            println!("\nNo GPU samples available for {timeframe_label}.");
        } else {
            println!(
                "\nGPU stats ({})\n{}",
                timeframe.label.replace('_', " "),
                gpu_stats_table(bucket_seconds, &usage_buckets, &freq_buckets)
            );
        }
    }

    if presets.contains(&ReportPreset::Memory) {
        let memory_buckets = bucket_usage_stats(metrics, MetricKind::MemoryUsage, bucket_seconds);
        if memory_buckets.is_empty() {
            println!("\nNo memory samples available for {timeframe_label}.");
        } else {
            println!(
                "\nMemory stats ({})\n{}",
                timeframe.label.replace('_', " "),
                memory_stats_table(bucket_seconds, &memory_buckets)
            );
        }
    }

    if presets.contains(&ReportPreset::Disk) {
        let disk_buckets = bucket_usage_stats(metrics, MetricKind::DiskUsage, bucket_seconds);
        if disk_buckets.is_empty() {
            println!("\nNo disk samples available for {timeframe_label}.");
        } else {
            println!(
                "\nDisk stats ({})\n{}",
                timeframe.label.replace('_', " "),
                disk_stats_table(bucket_seconds, &disk_buckets)
            );
        }
    }

    if presets.contains(&ReportPreset::Network) {
        let network_buckets = bucket_network_rates(&network_rates, bucket_seconds);
        if network_buckets.is_empty() {
            println!("\nNo network samples available for {timeframe_label}.");
        } else {
            println!(
                "\nNetwork stats ({})\n{}",
                timeframe.label.replace('_', " "),
                network_stats_table(bucket_seconds, &network_buckets)
            );
        }
    }

    if presets.contains(&ReportPreset::Temperature) {
        let temp_buckets =
            bucket_stats_for_kind_by_source(metrics, MetricKind::Temperature, bucket_seconds);
        if temp_buckets.is_empty() {
            println!("\nNo temperature samples available for {timeframe_label}.");
        } else {
            println!(
                "\nTemperature stats ({})\n{}",
                timeframe.label.replace('_', " "),
                temperature_stats_table(bucket_seconds, &temp_buckets)
            );
        }
    }
}

fn format_power(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.2}W"),
        None => "--".to_string(),
    }
}

fn themed_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL_CONDENSED)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

fn header_cells(labels: &[&str]) -> Vec<Cell> {
    labels
        .iter()
        .map(|label| {
            Cell::new(*label)
                .add_attribute(Attribute::Bold)
                .fg(Color::Cyan)
        })
        .collect()
}

fn label_cell(text: &str) -> Cell {
    Cell::new(text).add_attribute(Attribute::Bold)
}

fn value_cell<T: std::fmt::Display>(value: T) -> Cell {
    Cell::new(value.to_string()).set_alignment(CellAlignment::Right)
}

fn status_cell(status: Option<&str>) -> Cell {
    let status_text = status.unwrap_or("unknown");
    let color = match status_text.to_ascii_lowercase().as_str() {
        s if s.contains("charging") && !s.contains("dis") => Color::Green,
        s if s.contains("discharging") => Color::Yellow,
        s if s.contains("full") => Color::Blue,
        _ => Color::White,
    };
    Cell::new(status_text).fg(color)
}

fn format_percent(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.1}%"))
        .unwrap_or_else(|| "--".to_string())
}

fn format_rate(value: Option<f64>) -> String {
    value
        .map(|v| format!("{}/s", format_bytes(v)))
        .unwrap_or_else(|| "--".to_string())
}

#[derive(Default, Clone)]
struct NumberStats {
    total: f64,
    count: usize,
    min: f64,
    max: f64,
}

impl NumberStats {
    fn record(&mut self, value: f64) {
        if self.count == 0 {
            self.min = value;
            self.max = value;
        } else {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
        self.total += value;
        self.count += 1;
    }

    fn record_opt(&mut self, value: Option<f64>) {
        if let Some(v) = value {
            self.record(v);
        }
    }

    fn average(&self) -> Option<f64> {
        (self.count > 0).then_some(self.total / self.count as f64)
    }

    fn min(&self) -> Option<f64> {
        (self.count > 0).then_some(self.min)
    }

    fn max(&self) -> Option<f64> {
        (self.count > 0).then_some(self.max)
    }
}

#[derive(Default, Clone)]
struct UsageStats {
    used: NumberStats,
    percent: NumberStats,
}

impl UsageStats {
    fn record(&mut self, used: Option<f64>, total: Option<f64>) {
        if let Some(used_bytes) = used {
            self.used.record(used_bytes);
            if let Some(total_bytes) = total {
                if total_bytes > 0.0 {
                    self.percent.record((used_bytes / total_bytes) * 100.0);
                }
            }
        }
    }
}

#[derive(Default, Clone)]
struct RateStats {
    rx: NumberStats,
    tx: NumberStats,
}

impl RateStats {
    fn record(&mut self, rx: Option<f64>, tx: Option<f64>) {
        self.rx.record_opt(rx);
        self.tx.record_opt(tx);
    }
}

fn average_for_kind(metrics: &[MetricSample], kind: MetricKind) -> NumberStats {
    let mut stats = NumberStats::default();
    for sample in metrics.iter().filter(|s| s.kind == kind) {
        stats.record_opt(sample.value);
    }
    stats
}

fn filter_metrics_by_source(
    metrics: &[MetricSample],
    sensor_filters: &[String],
) -> Vec<MetricSample> {
    if sensor_filters.is_empty() {
        return metrics.to_vec();
    }
    metrics
        .iter()
        .filter(|m| sensor_filters.iter().any(|f| f == &m.source))
        .cloned()
        .collect()
}

type SourceBuckets = BTreeMap<String, BTreeMap<DateTime<Local>, NumberStats>>;

fn bucket_stats_for_kind_by_source(
    metrics: &[MetricSample],
    kind: MetricKind,
    bucket_seconds: i64,
) -> SourceBuckets {
    let mut buckets: SourceBuckets = BTreeMap::new();
    for sample in metrics.iter().filter(|s| s.kind == kind) {
        if let Some(value) = sample.value {
            let bucket = bucket_start(sample.ts, bucket_seconds);
            buckets
                .entry(sample.source.clone())
                .or_default()
                .entry(bucket)
                .or_default()
                .record(value);
        }
    }
    buckets
}

fn data_span_seconds(samples: &[Sample], metrics: &[MetricSample]) -> Option<f64> {
    let mut min_ts = f64::INFINITY;
    let mut max_ts = f64::NEG_INFINITY;

    for ts in samples
        .iter()
        .map(|s| s.ts)
        .chain(metrics.iter().map(|m| m.ts))
    {
        min_ts = min_ts.min(ts);
        max_ts = max_ts.max(ts);
    }

    if max_ts.is_finite() && min_ts.is_finite() && max_ts >= min_ts {
        Some(max_ts - min_ts)
    } else {
        None
    }
}

fn bucket_stats_for_kind(
    metrics: &[MetricSample],
    kind: MetricKind,
    bucket_seconds: i64,
) -> BTreeMap<DateTime<Local>, NumberStats> {
    let mut buckets: BTreeMap<DateTime<Local>, NumberStats> = BTreeMap::new();
    for sample in metrics.iter().filter(|s| s.kind == kind) {
        if let Some(value) = sample.value {
            let bucket = bucket_start(sample.ts, bucket_seconds);
            buckets.entry(bucket).or_default().record(value);
        }
    }
    buckets
}

#[cfg(test)]
fn usage_stats_for_kind(metrics: &[MetricSample], kind: MetricKind) -> UsageStats {
    let mut stats = UsageStats::default();
    for sample in metrics.iter().filter(|s| s.kind == kind) {
        let total = number_from_details(sample, "total_bytes");
        stats.record(sample.value, total);
    }
    stats
}

fn bucket_usage_stats(
    metrics: &[MetricSample],
    kind: MetricKind,
    bucket_seconds: i64,
) -> BTreeMap<DateTime<Local>, UsageStats> {
    let mut buckets: BTreeMap<DateTime<Local>, UsageStats> = BTreeMap::new();
    for sample in metrics.iter().filter(|s| s.kind == kind) {
        let bucket = bucket_start(sample.ts, bucket_seconds);
        let total = number_from_details(sample, "total_bytes");
        buckets
            .entry(bucket)
            .or_default()
            .record(sample.value, total);
    }
    buckets
}

struct NetworkRateSample {
    ts: f64,
    rx_rate: Option<f64>,
    tx_rate: Option<f64>,
}

fn rate_from_counters(previous: Option<f64>, current: Option<f64>, dt: f64) -> Option<f64> {
    match (previous, current) {
        (Some(prev), Some(next)) if next >= prev && dt > 0.0 => Some((next - prev) / dt),
        _ => None,
    }
}

fn compute_network_rates(metrics: &[MetricSample]) -> Vec<NetworkRateSample> {
    let mut by_iface: BTreeMap<&str, Vec<&MetricSample>> = BTreeMap::new();
    for sample in metrics
        .iter()
        .filter(|s| s.kind == MetricKind::NetworkBytes)
    {
        by_iface.entry(&sample.source).or_default().push(sample);
    }

    let mut rates = Vec::new();
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
                number_from_details(prev, "rx_bytes"),
                number_from_details(next, "rx_bytes"),
                dt,
            );
            let tx_rate = rate_from_counters(
                number_from_details(prev, "tx_bytes"),
                number_from_details(next, "tx_bytes"),
                dt,
            );
            if rx_rate.is_none() && tx_rate.is_none() {
                continue;
            }
            rates.push(NetworkRateSample {
                ts: next.ts,
                rx_rate,
                tx_rate,
            });
        }
    }
    rates
}

fn bucket_network_rates(
    rates: &[NetworkRateSample],
    bucket_seconds: i64,
) -> BTreeMap<DateTime<Local>, RateStats> {
    let mut buckets: BTreeMap<DateTime<Local>, RateStats> = BTreeMap::new();
    for rate in rates {
        let bucket = bucket_start(rate.ts, bucket_seconds);
        buckets
            .entry(bucket)
            .or_default()
            .record(rate.rx_rate, rate.tx_rate);
    }
    buckets
}

fn format_freq(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.0}MHz"))
        .unwrap_or_else(|| "--".to_string())
}

fn battery_summary_table(
    timeframe_records: usize,
    avg_discharge_w: Option<f64>,
    avg_charge_w: Option<f64>,
    est_runtime_hours: Option<f64>,
) -> Table {
    let mut table = themed_table();
    table.set_header(header_cells(&["Metric", "Value"]));
    table.add_row(vec![
        label_cell("Records in window"),
        value_cell(timeframe_records),
    ]);
    table.add_row(vec![
        label_cell("Avg discharge power"),
        value_cell(format_power(avg_discharge_w)),
    ]);
    table.add_row(vec![
        label_cell("Avg charge power"),
        value_cell(format_power(avg_charge_w)),
    ]);
    table.add_row(vec![
        label_cell("Est runtime (full)"),
        value_cell(format_runtime(est_runtime_hours)),
    ]);
    table
}

fn battery_stats_table(
    samples: &[Sample],
    power_draw: &BTreeMap<DateTime<Local>, NumberStats>,
    bucket_seconds: i64,
) -> Table {
    let mut buckets: BTreeMap<DateTime<Local>, Vec<&Sample>> = BTreeMap::new();
    for sample in samples {
        let bucket_key = bucket_start(sample.ts, bucket_seconds);
        buckets.entry(bucket_key).or_default().push(sample);
    }

    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Window",
        "Records",
        "Min %",
        "Avg %",
        "Max %",
        "Avg discharge W",
        "Avg charge W",
        "Latest status",
    ]));

    for (bucket_start, bucket_samples) in buckets {
        let pct_values: Vec<f64> = bucket_samples.iter().filter_map(|s| s.percentage).collect();
        let (min_pct, avg_pct, max_pct) = pct_stats(&pct_values);
        let latest_status = bucket_samples
            .last()
            .and_then(|s| s.status.as_deref())
            .unwrap_or("unknown");
        let rates = average_rates(bucket_samples.iter().copied());
        let draw = power_draw
            .get(&bucket_start)
            .and_then(NumberStats::average)
            .or(rates.discharge_w);
        report.add_row(vec![
            Cell::new(format_bucket(bucket_start, bucket_seconds))
                .fg(Color::Magenta)
                .add_attribute(Attribute::Bold),
            value_cell(bucket_samples.len()),
            value_cell(min_pct),
            value_cell(avg_pct),
            value_cell(max_pct),
            value_cell(format_power(draw)),
            value_cell(format_power(rates.charge_w)),
            status_cell(Some(latest_status)),
        ]);
    }
    report
}

fn cpu_stats_table(bucket_seconds: i64, usage: &SourceBuckets, freq: &SourceBuckets) -> Table {
    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Source",
        "Window",
        "Samples",
        "Min usage",
        "Avg usage",
        "Peak usage",
        "Min freq",
        "Avg freq",
        "Peak freq",
    ]));

    let mut sources: Vec<&String> = usage.keys().chain(freq.keys()).collect();
    sources.sort();
    sources.dedup();

    for source in sources {
        let usage_buckets = usage.get(source);
        let freq_buckets = freq.get(source);
        let mut keys: Vec<DateTime<Local>> = usage_buckets
            .into_iter()
            .flat_map(|m| m.keys().copied())
            .chain(freq_buckets.into_iter().flat_map(|m| m.keys().copied()))
            .collect();
        keys.sort();
        keys.dedup();

        for key in keys {
            let usage_stats = usage_buckets
                .and_then(|map| map.get(&key).cloned())
                .unwrap_or_default();
            let freq_stats = freq_buckets
                .and_then(|map| map.get(&key).cloned())
                .unwrap_or_default();
            let samples = usage_stats.count.max(freq_stats.count);
            report.add_row(vec![
                label_cell(source),
                Cell::new(format_bucket(key, bucket_seconds))
                    .fg(Color::Magenta)
                    .add_attribute(Attribute::Bold),
                value_cell(samples),
                value_cell(format_percent(usage_stats.min())),
                value_cell(format_percent(usage_stats.average())),
                value_cell(format_percent(usage_stats.max())),
                value_cell(format_freq(freq_stats.min())),
                value_cell(format_freq(freq_stats.average())),
                value_cell(format_freq(freq_stats.max())),
            ]);
        }
    }
    report
}

fn gpu_stats_table(bucket_seconds: i64, usage: &SourceBuckets, freq: &SourceBuckets) -> Table {
    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Source",
        "Window",
        "Samples",
        "Min usage",
        "Avg usage",
        "Peak usage",
        "Min freq",
        "Avg freq",
        "Peak freq",
    ]));

    let mut sources: Vec<&String> = usage.keys().chain(freq.keys()).collect();
    sources.sort();
    sources.dedup();

    for source in sources {
        let usage_buckets = usage.get(source);
        let freq_buckets = freq.get(source);
        let mut keys: Vec<DateTime<Local>> = usage_buckets
            .into_iter()
            .flat_map(|m| m.keys().copied())
            .chain(freq_buckets.into_iter().flat_map(|m| m.keys().copied()))
            .collect();
        keys.sort();
        keys.dedup();

        for key in keys {
            let usage_stats = usage_buckets
                .and_then(|map| map.get(&key).cloned())
                .unwrap_or_default();
            let freq_stats = freq_buckets
                .and_then(|map| map.get(&key).cloned())
                .unwrap_or_default();
            let samples = usage_stats.count.max(freq_stats.count);
            report.add_row(vec![
                label_cell(source),
                Cell::new(format_bucket(key, bucket_seconds))
                    .fg(Color::Magenta)
                    .add_attribute(Attribute::Bold),
                value_cell(samples),
                value_cell(format_percent(usage_stats.min())),
                value_cell(format_percent(usage_stats.average())),
                value_cell(format_percent(usage_stats.max())),
                value_cell(format_freq(freq_stats.min())),
                value_cell(format_freq(freq_stats.average())),
                value_cell(format_freq(freq_stats.max())),
            ]);
        }
    }
    report
}

fn memory_stats_table(
    bucket_seconds: i64,
    buckets: &BTreeMap<DateTime<Local>, UsageStats>,
) -> Table {
    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Window",
        "Samples",
        "Min used",
        "Avg used",
        "Min used %",
        "Avg used %",
        "Peak used %",
    ]));

    for (key, stats) in buckets {
        report.add_row(vec![
            Cell::new(format_bucket(*key, bucket_seconds))
                .fg(Color::Magenta)
                .add_attribute(Attribute::Bold),
            value_cell(stats.used.count),
            value_cell(format_opt_bytes(stats.used.min())),
            value_cell(format_opt_bytes(stats.used.average())),
            value_cell(format_percent(stats.percent.min())),
            value_cell(format_percent(stats.percent.average())),
            value_cell(format_percent(stats.percent.max())),
        ]);
    }
    report
}

fn disk_stats_table(bucket_seconds: i64, buckets: &BTreeMap<DateTime<Local>, UsageStats>) -> Table {
    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Window",
        "Samples",
        "Min used",
        "Avg used",
        "Min used %",
        "Avg used %",
        "Peak used %",
    ]));

    for (key, stats) in buckets {
        report.add_row(vec![
            Cell::new(format_bucket(*key, bucket_seconds))
                .fg(Color::Magenta)
                .add_attribute(Attribute::Bold),
            value_cell(stats.used.count),
            value_cell(format_opt_bytes(stats.used.min())),
            value_cell(format_opt_bytes(stats.used.average())),
            value_cell(format_percent(stats.percent.min())),
            value_cell(format_percent(stats.percent.average())),
            value_cell(format_percent(stats.percent.max())),
        ]);
    }
    report
}

fn temperature_stats_table(bucket_seconds: i64, buckets: &SourceBuckets) -> Table {
    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Source",
        "Window",
        "Samples",
        "Min temp",
        "Avg temp",
        "Peak temp",
    ]));

    for (source, readings) in buckets {
        for (key, stats) in readings {
            report.add_row(vec![
                label_cell(source),
                Cell::new(format_bucket(*key, bucket_seconds))
                    .fg(Color::Magenta)
                    .add_attribute(Attribute::Bold),
                value_cell(stats.count),
                value_cell(
                    stats
                        .min()
                        .map(|v| format!("{v:.1}C"))
                        .unwrap_or_else(|| "--".to_string()),
                ),
                value_cell(
                    stats
                        .average()
                        .map(|v| format!("{v:.1}C"))
                        .unwrap_or_else(|| "--".to_string()),
                ),
                value_cell(
                    stats
                        .max()
                        .map(|v| format!("{v:.1}C"))
                        .unwrap_or_else(|| "--".to_string()),
                ),
            ]);
        }
    }
    report
}

fn network_stats_table(
    bucket_seconds: i64,
    buckets: &BTreeMap<DateTime<Local>, RateStats>,
) -> Table {
    let mut report = themed_table();
    report.set_header(header_cells(&[
        "Window",
        "Samples",
        "Avg down",
        "Peak down",
        "Avg up",
        "Peak up",
    ]));

    for (key, stats) in buckets {
        let samples = stats.rx.count.max(stats.tx.count);
        report.add_row(vec![
            Cell::new(format_bucket(*key, bucket_seconds))
                .fg(Color::Magenta)
                .add_attribute(Attribute::Bold),
            value_cell(samples),
            value_cell(format_rate(stats.rx.average())),
            value_cell(format_rate(stats.rx.max())),
            value_cell(format_rate(stats.tx.average())),
            value_cell(format_rate(stats.tx.max())),
        ]);
    }
    report
}

fn format_bytes(value: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut val = value;
    let mut unit = "B";
    for next in &UNITS {
        unit = next;
        if val.abs() < 1024.0 || *next == "TiB" {
            break;
        }
        val /= 1024.0;
    }
    if unit == "B" {
        format!("{val:.0}{unit}")
    } else {
        format!("{val:.1}{unit}")
    }
}

fn format_opt_bytes(value: Option<f64>) -> String {
    value.map(format_bytes).unwrap_or_else(|| "--".to_string())
}

fn number_from_details(sample: &MetricSample, key: &str) -> Option<f64> {
    sample
        .details
        .get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn pct_stats(values: &[f64]) -> (String, String, String) {
    if values.is_empty() {
        return ("--".to_string(), "--".to_string(), "--".to_string());
    }
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    (
        format!("{min:.1}%"),
        format!("{avg:.1}%"),
        format!("{max:.1}%"),
    )
}

fn format_bucket(dt: DateTime<Local>, bucket_seconds: i64) -> String {
    if bucket_seconds < 3600 {
        dt.format("%m-%d %H:%M").to_string()
    } else if bucket_seconds < 24 * 3600 {
        dt.format("%m-%d %H:00").to_string()
    } else {
        let days = bucket_seconds / (24 * 3600);
        if days <= 1 {
            dt.format("%Y-%m-%d").to_string()
        } else {
            format!("{} (+{days}d)", dt.format("%Y-%m-%d"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metric_sample_with_source(
        kind: MetricKind,
        source: &str,
        ts: f64,
        value: Option<f64>,
        details: serde_json::Value,
    ) -> MetricSample {
        MetricSample {
            ts,
            kind,
            source: source.to_string(),
            value,
            unit: None,
            details,
        }
    }

    fn metric_sample(
        kind: MetricKind,
        ts: f64,
        value: Option<f64>,
        details: serde_json::Value,
    ) -> MetricSample {
        metric_sample_with_source(kind, "test", ts, value, details)
    }

    #[test]
    fn network_rates_compute_per_second() {
        let metrics = vec![
            metric_sample(
                MetricKind::NetworkBytes,
                0.0,
                Some(1000.0),
                json!({"rx_bytes": 1_000.0, "tx_bytes": 500.0}),
            ),
            metric_sample(
                MetricKind::NetworkBytes,
                10.0,
                Some(4000.0),
                json!({"rx_bytes": 3_000.0, "tx_bytes": 1_500.0}),
            ),
        ];

        let rates = compute_network_rates(&metrics);
        assert_eq!(rates.len(), 1);
        let rate = &rates[0];
        assert!((rate.rx_rate.unwrap() - 200.0).abs() < 1e-6);
        assert!((rate.tx_rate.unwrap() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn bucket_stats_are_kept_per_source() {
        let metrics = vec![
            metric_sample_with_source(MetricKind::Temperature, "cpu0", 0.0, Some(30.0), json!({})),
            metric_sample_with_source(MetricKind::Temperature, "cpu1", 0.0, Some(40.0), json!({})),
            metric_sample_with_source(MetricKind::Temperature, "cpu0", 60.0, Some(50.0), json!({})),
        ];

        let buckets = bucket_stats_for_kind_by_source(&metrics, MetricKind::Temperature, 60);
        assert_eq!(buckets.len(), 2);
        let cpu0_count: usize = buckets
            .get("cpu0")
            .unwrap()
            .values()
            .map(|stats| stats.count)
            .sum();
        let cpu1_count: usize = buckets
            .get("cpu1")
            .unwrap()
            .values()
            .map(|stats| stats.count)
            .sum();
        assert_eq!(cpu0_count, 2);
        assert_eq!(cpu1_count, 1);
    }

    #[test]
    fn metrics_can_be_filtered_by_source() {
        let metrics = vec![
            metric_sample_with_source(MetricKind::CpuUsage, "cpu0", 0.0, Some(10.0), json!({})),
            metric_sample_with_source(MetricKind::CpuUsage, "cpu1", 0.0, Some(20.0), json!({})),
        ];

        let filtered = filter_metrics_by_source(&metrics, &["cpu1".to_string()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].source, "cpu1");
    }

    #[test]
    fn usage_stats_compute_percentage() {
        let metrics = vec![metric_sample(
            MetricKind::MemoryUsage,
            0.0,
            Some(2048.0),
            json!({"total_bytes": 4096.0}),
        )];

        let stats = usage_stats_for_kind(&metrics, MetricKind::MemoryUsage);
        assert_eq!(stats.used.count, 1);
        assert!((stats.used.average().unwrap() - 2048.0).abs() < 1e-6);
        assert!((stats.percent.average().unwrap() - 50.0).abs() < 1e-6);
    }
}
