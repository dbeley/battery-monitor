use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use log::{info, warn};
use plotters::prelude::*;
use plotters::series::LineSeries;
use plotters::style::RGBColor;

use crate::aggregate::aggregate_samples_by_timestamp;
use crate::db::{self, Sample};
use crate::timeframe::Timeframe;

const ORANGE: RGBColor = RGBColor(255, 127, 14);

pub fn load_series(db_path: &Path, timeframe: &Timeframe) -> Result<Vec<Sample>> {
    let since_ts = timeframe.since_timestamp(None);
    let raw = db::fetch_samples(db_path, since_ts)?;
    Ok(aggregate_samples_by_timestamp(&raw))
}

pub fn render_plot(samples: &[Sample], timeframe: &Timeframe, output: &Path) -> Result<()> {
    if samples.is_empty() {
        warn!("No records to plot");
        return Ok(());
    }

    let mut percent_points: Vec<(DateTime<Utc>, f64)> = Vec::new();
    let mut health_points: Vec<(DateTime<Utc>, f64)> = Vec::new();
    for sample in samples {
        let ts = Utc.timestamp_opt(sample.ts as i64, 0).unwrap();
        if let Some(value) = sample.percentage {
            percent_points.push((ts, value));
        }
        if let Some(value) = sample.health_pct {
            health_points.push((ts, value));
        }
    }

    if percent_points.is_empty() && health_points.is_empty() {
        warn!("No charge or health values present to plot");
        return Ok(());
    }

    let min_ts = percent_points
        .iter()
        .chain(&health_points)
        .map(|(ts, _)| *ts)
        .min()
        .unwrap();
    let max_ts = percent_points
        .iter()
        .chain(&health_points)
        .map(|(ts, _)| *ts)
        .max()
        .unwrap();

    let caption = format!("Battery ({})", timeframe.label.replace('_', " "));

    let root = BitMapBackend::new(output, (1280, 720)).into_drawing_area();
    root.fill(&WHITE)?;
    let mut chart = ChartBuilder::on(&root)
        .caption(caption, ("sans-serif", 22).into_font())
        .margin(10)
        .x_label_area_size(40)
        .y_label_area_size(50)
        .build_cartesian_2d(min_ts..max_ts, 0f64..110f64)?;

    chart
        .configure_mesh()
        .x_labels(6)
        .y_labels(11)
        .x_desc("Time")
        .y_desc("Percent")
        .light_line_style(WHITE.mix(0.2))
        .draw()?;

    if !percent_points.is_empty() {
        chart
            .draw_series(LineSeries::new(percent_points.clone(), &BLUE))?
            .label("Charge %")
            .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 10, y)], BLUE));
    }
    if !health_points.is_empty() {
        chart
            .draw_series(LineSeries::new(health_points.clone(), &ORANGE))?
            .label("Health %")
            .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 10, y)], ORANGE));
    }

    chart
        .configure_series_labels()
        .background_style(WHITE.mix(0.8))
        .border_style(BLACK)
        .draw()?;

    root.present()?;
    info!("Saved plot to {}", output.display());
    Ok(())
}
