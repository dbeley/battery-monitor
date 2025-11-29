use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{Attribute, Cell, CellAlignment, Color, ContentArrangement, Table};

use chrono::{DateTime, Local, TimeZone};

use crate::aggregate::{aggregate_group, aggregate_samples_by_timestamp};
use crate::cli_helpers::{
    average_rates, bucket_span_seconds, bucket_start, default_graph_path, estimate_runtime_hours,
    format_runtime,
};
use crate::collector::{collect_loop, collect_once, resolve_db_path};
use crate::db::{self, Sample};
use crate::graph;
use crate::timeframe::{build_timeframe, Timeframe};

#[derive(Parser)]
#[command(name = "battery-monitor", version)]
#[command(about = "Battery monitoring tools for Linux/NixOS")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Collect battery metrics once (or repeatedly with --interval)
    Collect {
        /// Path to SQLite database (or set BATTERY_MONITOR_DB)
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
            verbose,
        } => {
            configure_logging(verbose);
            let timeframe = build_timeframe(hours as i64, days as i64, months as i64, all_time)?;
            let resolved = resolve_db_path(db_path.as_deref());

            let total_records = db::count_samples(&resolved, None)?;
            if total_records == 0 {
                println!("No records available; collect data first.");
                std::process::exit(1);
            }

            let since_ts = timeframe.since_timestamp(None);
            let raw_samples = db::fetch_samples(&resolved, since_ts)?;
            let samples = aggregate_samples_by_timestamp(&raw_samples);
            if samples.is_empty() {
                println!(
                    "No records for {}; try a broader timeframe.",
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
                graph::render_plot(&samples, &timeframe, &path)?;
            }

            let first_event = db::fetch_first_event(&resolved)?;
            let first_sample = if first_event.is_empty() {
                None
            } else {
                Some(aggregate_group(&first_event)?)
            };
            let latest_event = db::fetch_latest_event(&resolved)?;
            let latest_sample = if latest_event.is_empty() {
                samples.last().cloned().unwrap()
            } else {
                aggregate_group(&latest_event)?
            };
            let recent_events = db::fetch_recent_events(&resolved, 5)?;
            let mut recent_samples = Vec::new();
            for event in recent_events {
                if let Ok(sample) = aggregate_group(&event) {
                    recent_samples.push(sample);
                }
            }
            summarize(
                &samples,
                &timeframe,
                total_records,
                first_sample.as_ref(),
                &latest_sample,
                &recent_samples,
            );
        }
    }
    Ok(())
}

fn summarize(
    timeframe_samples: &[Sample],
    timeframe: &Timeframe,
    total_records: usize,
    first_sample: Option<&Sample>,
    latest_sample: &Sample,
    recent_samples: &[Sample],
) {
    let timeframe_label = timeframe.label.replace('_', " ");
    let rates = average_rates(timeframe_samples);
    let avg_discharge_w = rates.discharge_w;
    let avg_charge_w = rates.charge_w;
    let est_runtime_hours = estimate_runtime_hours(avg_discharge_w, latest_sample);

    let mut summary = themed_table();
    summary.set_header(header_cells(&["Field", "Value"]));
    summary.add_row(vec![label_cell("Records (all)"), value_cell(total_records)]);
    let first_ts = first_sample.map(|s| s.ts).unwrap_or(latest_sample.ts);
    summary.add_row(vec![
        label_cell("First record ts"),
        value_cell(format_timestamp(first_ts)),
    ]);
    summary.add_row(vec![
        label_cell("Latest record ts"),
        value_cell(format_timestamp(latest_sample.ts)),
    ]);
    summary.add_row(vec![
        label_cell("Timeframe window"),
        value_cell(timeframe_label),
    ]);
    summary.add_row(vec![
        label_cell("Latest status"),
        status_cell(latest_sample.status.as_deref()),
    ]);
    summary.add_row(vec![
        label_cell("Avg discharge power"),
        value_cell(format_power(avg_discharge_w)),
    ]);
    summary.add_row(vec![
        label_cell("Avg charge power"),
        value_cell(format_power(avg_charge_w)),
    ]);
    summary.add_row(vec![
        label_cell("Est runtime (full)"),
        value_cell(format_runtime(est_runtime_hours)),
    ]);
    println!("\nSummary\n{summary}");

    println!("\nRecent events\n{}", recent_table(recent_samples));
    println!("\nLatest reading\n{}", latest_table(latest_sample));
    println!(
        "\nTimeframe buckets ({})\n{}",
        timeframe.label.replace('_', " "),
        timeframe_report_table(timeframe, timeframe_samples)
    );
}

fn format_timestamp(ts: f64) -> String {
    let dt = Local.timestamp_opt(ts as i64, 0).unwrap();
    dt.format("%Y-%m-%d %H:%M:%S %Z").to_string()
}

fn format_power(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.2}W"),
        None => "--".to_string(),
    }
}

fn format_number(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.2}"),
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

fn pct_cell(value: Option<f64>) -> Cell {
    match value {
        Some(v) => {
            let color = if v < 20.0 {
                Color::Red
            } else if v < 50.0 {
                Color::Yellow
            } else {
                Color::Green
            };
            value_cell(format!("{v:.1}%")).fg(color)
        }
        None => value_cell("--"),
    }
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

fn latest_table(sample: &Sample) -> Table {
    let mut latest = themed_table();
    latest.set_header(header_cells(&["Metric", "Value"]));
    latest.add_row(vec![label_cell("Charge %"), pct_cell(sample.percentage)]);
    latest.add_row(vec![label_cell("Health %"), pct_cell(sample.health_pct)]);
    latest.add_row(vec![
        label_cell("Capacity %"),
        pct_cell(sample.capacity_pct),
    ]);
    latest.add_row(vec![
        label_cell("Energy now (Wh)"),
        value_cell(format_number(sample.energy_now_wh)),
    ]);
    latest.add_row(vec![
        label_cell("Energy full (Wh)"),
        value_cell(format_number(sample.energy_full_wh)),
    ]);
    latest.add_row(vec![
        label_cell("Energy design (Wh)"),
        value_cell(format_number(sample.energy_full_design_wh)),
    ]);
    latest.add_row(vec![label_cell("Source"), Cell::new(&sample.source_path)]);
    latest
}

fn recent_table(samples: &[Sample]) -> Table {
    let mut table = themed_table();
    table.set_header(header_cells(&[
        "When", "Charge", "Health", "Status", "Source",
    ]));
    for sample in samples {
        let when = Local.timestamp_opt(sample.ts as i64, 0).unwrap();
        table.add_row(vec![
            Cell::new(when.format("%m-%d %H:%M").to_string())
                .fg(Color::Magenta)
                .add_attribute(Attribute::Bold),
            pct_cell(sample.percentage),
            pct_cell(sample.health_pct),
            status_cell(sample.status.as_deref()),
            Cell::new(
                sample
                    .source_path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&sample.source_path)
                    .to_string(),
            )
            .fg(Color::Cyan),
        ]);
    }
    table
}

fn timeframe_report_table(timeframe: &Timeframe, samples: &[Sample]) -> Table {
    let bucket_seconds = bucket_span_seconds(timeframe);
    let mut buckets: std::collections::BTreeMap<DateTime<Local>, Vec<&Sample>> =
        std::collections::BTreeMap::new();
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
        report.add_row(vec![
            Cell::new(format_bucket(bucket_start, bucket_seconds))
                .fg(Color::Magenta)
                .add_attribute(Attribute::Bold),
            value_cell(bucket_samples.len()),
            value_cell(min_pct),
            value_cell(avg_pct),
            value_cell(max_pct),
            value_cell(format_power(rates.discharge_w)),
            value_cell(format_power(rates.charge_w)),
            status_cell(Some(latest_status)),
        ]);
    }
    report
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
