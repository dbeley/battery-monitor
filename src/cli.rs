use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
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

            if db::count_samples(&resolved, None)? == 0 {
                println!("No records available; collect data first.");
                std::process::exit(1);
            }

            let since_ts = timeframe.since_timestamp(None);
            let raw_samples = db::fetch_samples(&resolved, since_ts)?;
            let timeframe_record_count = raw_samples.len();
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

            summarize(&samples, &timeframe, timeframe_record_count);
        }
    }
    Ok(())
}

fn summarize(timeframe_samples: &[Sample], timeframe: &Timeframe, timeframe_records: usize) {
    let timeframe_label = timeframe.label.replace('_', " ");
    let rates = average_rates(timeframe_samples);
    let latest_sample = timeframe_samples
        .last()
        .expect("timeframe_samples should never be empty");
    let est_runtime_hours = estimate_runtime_hours(rates.discharge_w, latest_sample);

    println!(
        "\nTimeframe summary ({})\n{}",
        timeframe_label,
        timeframe_summary_table(
            timeframe_records,
            rates.discharge_w,
            rates.charge_w,
            rates.power_consumption_w,
            est_runtime_hours
        )
    );
    println!(
        "\nTimeframe windows ({})\n{}",
        timeframe.label.replace('_', " "),
        timeframe_report_table(timeframe, timeframe_samples)
    );
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

fn timeframe_summary_table(
    timeframe_records: usize,
    avg_discharge_w: Option<f64>,
    avg_charge_w: Option<f64>,
    avg_power_consumption_w: Option<f64>,
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
        label_cell("Avg power consumption"),
        value_cell(format_power(avg_power_consumption_w)),
    ]);
    table.add_row(vec![
        label_cell("Est runtime (full)"),
        value_cell(format_runtime(est_runtime_hours)),
    ]);
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
        "Avg power W",
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
            value_cell(format_power(rates.power_consumption_w)),
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
