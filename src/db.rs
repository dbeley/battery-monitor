use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{params, Connection, Row};

use crate::sysfs::BatteryReading;

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub ts: f64,
    pub percentage: Option<f64>,
    pub capacity_pct: Option<f64>,
    pub health_pct: Option<f64>,
    pub energy_now_wh: Option<f64>,
    pub energy_full_wh: Option<f64>,
    pub energy_full_design_wh: Option<f64>,
    pub status: Option<String>,
    pub source_path: String,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS samples (
    ts REAL NOT NULL,
    percentage REAL,
    capacity_pct REAL,
    health_pct REAL,
    energy_now_wh REAL,
    energy_full_wh REAL,
    energy_full_design_wh REAL,
    status TEXT,
    source_path TEXT
);
CREATE INDEX IF NOT EXISTS idx_samples_ts ON samples (ts);
"#;

pub fn init_db(db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

pub fn insert_sample(db_path: &Path, sample: &Sample) -> Result<()> {
    insert_samples(db_path, std::slice::from_ref(sample))
}

pub fn insert_samples(db_path: &Path, samples: &[Sample]) -> Result<()> {
    let mut conn = Connection::open(db_path)?;
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            r#"
            INSERT INTO samples (
                ts, percentage, capacity_pct, health_pct, energy_now_wh,
                energy_full_wh, energy_full_design_wh, status, source_path
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )?;
        for sample in samples {
            stmt.execute(params![
                sample.ts,
                sample.percentage,
                sample.capacity_pct,
                sample.health_pct,
                sample.energy_now_wh,
                sample.energy_full_wh,
                sample.energy_full_design_wh,
                sample.status,
                sample.source_path,
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn count_samples(db_path: &Path, since_ts: Option<f64>) -> Result<usize> {
    let conn = Connection::open(db_path)?;
    let count: i64 = match since_ts {
        Some(ts) => conn.query_row(
            "SELECT COUNT(*) FROM samples WHERE ts >= ?",
            params![ts],
            |row| row.get(0),
        )?,
        None => conn.query_row("SELECT COUNT(*) FROM samples", [], |row| row.get(0))?,
    };
    Ok(count as usize)
}

pub fn count_events(db_path: &Path, since_ts: Option<f64>) -> Result<usize> {
    let conn = Connection::open(db_path)?;
    let count: i64 = match since_ts {
        Some(ts) => conn.query_row(
            "SELECT COUNT(DISTINCT ts) FROM samples WHERE ts >= ?",
            params![ts],
            |row| row.get(0),
        )?,
        None => conn.query_row("SELECT COUNT(DISTINCT ts) FROM samples", [], |row| {
            row.get(0)
        })?,
    };
    Ok(count as usize)
}

fn sample_from_row(row: &Row) -> rusqlite::Result<Sample> {
    Ok(Sample {
        ts: row.get("ts")?,
        percentage: row.get("percentage")?,
        capacity_pct: row.get("capacity_pct")?,
        health_pct: row.get("health_pct")?,
        energy_now_wh: row.get("energy_now_wh")?,
        energy_full_wh: row.get("energy_full_wh")?,
        energy_full_design_wh: row.get("energy_full_design_wh")?,
        status: row.get("status")?,
        source_path: row
            .get::<_, Option<String>>("source_path")?
            .unwrap_or_default(),
    })
}

pub fn fetch_samples(db_path: &Path, since_ts: Option<f64>) -> Result<Vec<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare(if since_ts.is_some() {
        "SELECT * FROM samples WHERE ts >= ? ORDER BY ts"
    } else {
        "SELECT * FROM samples ORDER BY ts"
    })?;
    let rows = match since_ts {
        Some(ts) => stmt.query_map(params![ts], sample_from_row)?,
        None => stmt.query_map([], sample_from_row)?,
    };
    let mut samples = Vec::new();
    for row in rows {
        samples.push(row?);
    }
    Ok(samples)
}

pub fn fetch_samples_for_timestamp(db_path: &Path, ts: f64) -> Result<Vec<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM samples WHERE ts = ? ORDER BY source_path")?;
    let rows = stmt.query_map(params![ts], sample_from_row)?;
    let mut samples = Vec::new();
    for row in rows {
        samples.push(row?);
    }
    Ok(samples)
}

pub fn fetch_first_sample(db_path: &Path) -> Result<Option<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM samples ORDER BY ts ASC LIMIT 1")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        Ok(Some(sample_from_row(row)?))
    } else {
        Ok(None)
    }
}

pub fn fetch_latest_sample(db_path: &Path) -> Result<Option<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM samples ORDER BY ts DESC LIMIT 1")?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        Ok(Some(sample_from_row(row)?))
    } else {
        Ok(None)
    }
}

pub fn fetch_recent_samples(db_path: &Path, limit: usize) -> Result<Vec<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT * FROM samples ORDER BY ts DESC LIMIT ?")?;
    let rows = stmt.query_map(params![limit as i64], sample_from_row)?;
    let mut samples = Vec::new();
    for row in rows {
        samples.push(row?);
    }
    Ok(samples)
}

pub fn fetch_first_event(db_path: &Path) -> Result<Vec<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT ts FROM samples ORDER BY ts ASC LIMIT 1")?;
    let mut rows = stmt.query([])?;
    let ts_row = match rows.next()? {
        Some(row) => row,
        None => return Ok(Vec::new()),
    };
    let ts: f64 = ts_row.get(0)?;
    fetch_samples_for_timestamp(db_path, ts)
}

pub fn fetch_latest_event(db_path: &Path) -> Result<Vec<Sample>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT ts FROM samples ORDER BY ts DESC LIMIT 1")?;
    let mut rows = stmt.query([])?;
    let ts_row = match rows.next()? {
        Some(row) => row,
        None => return Ok(Vec::new()),
    };
    let ts: f64 = ts_row.get(0)?;
    fetch_samples_for_timestamp(db_path, ts)
}

pub fn fetch_recent_events(db_path: &Path, limit: usize) -> Result<Vec<Vec<Sample>>> {
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT ts FROM samples GROUP BY ts ORDER BY ts DESC LIMIT ?")?;
    let mut rows = stmt.query(params![limit as i64])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let ts: f64 = row.get(0)?;
        events.push(fetch_samples_for_timestamp(db_path, ts)?);
    }
    Ok(events)
}

pub fn create_sample_from_reading(reading: &BatteryReading, ts: Option<f64>) -> Sample {
    let now = ts.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    });
    Sample {
        ts: now,
        percentage: reading.percentage,
        capacity_pct: reading.capacity_pct,
        health_pct: reading.health_pct,
        energy_now_wh: reading.energy_now_wh,
        energy_full_wh: reading.energy_full_wh,
        energy_full_design_wh: reading.energy_full_design_wh,
        status: reading.status.clone(),
        source_path: reading.path.to_string_lossy().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("battery.db");
        let reading = BatteryReading {
            path: tmp.path().join("BAT0"),
            capacity_pct: Some(90.0),
            percentage: Some(75.0),
            energy_now_wh: Some(50.0),
            energy_full_wh: Some(70.0),
            energy_full_design_wh: Some(80.0),
            health_pct: Some(87.5),
            status: Some("Discharging".to_string()),
        };
        let ts = 1_700_000_000.5;
        let sample = create_sample_from_reading(&reading, Some(ts));

        init_db(&db_path).unwrap();
        insert_sample(&db_path, &sample).unwrap();

        let rows = fetch_samples(&db_path, None).unwrap();
        assert_eq!(rows.len(), 1);
        let stored = &rows[0];
        assert_eq!(stored.ts, ts);
        assert_eq!(stored.percentage, Some(75.0));
        assert_eq!(stored.health_pct, Some(87.5));
        assert_eq!(stored.status.as_deref(), Some("Discharging"));
    }

    #[test]
    fn insert_samples_bulk() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("battery.db");
        let readings: Vec<BatteryReading> = (0..2)
            .map(|i| BatteryReading {
                path: tmp.path().join(format!("BAT{i}")),
                capacity_pct: Some(90.0 + i as f64),
                percentage: Some(70.0 + i as f64),
                energy_now_wh: Some(40.0 + i as f64),
                energy_full_wh: Some(60.0 + i as f64),
                energy_full_design_wh: Some(80.0 + i as f64),
                health_pct: Some(85.0 + i as f64),
                status: Some("Charging".to_string()),
            })
            .collect();
        let ts = 1_700_000_000.0;
        let samples: Vec<_> = readings
            .iter()
            .map(|reading| create_sample_from_reading(reading, Some(ts)))
            .collect();

        init_db(&db_path).unwrap();
        insert_samples(&db_path, &samples).unwrap();

        let rows = fetch_samples(&db_path, None).unwrap();
        assert_eq!(rows.len(), 2);
        let paths: Vec<String> = rows.iter().map(|r| r.source_path.clone()).collect();
        let expected: Vec<String> = readings
            .iter()
            .map(|r| r.path.to_string_lossy().to_string())
            .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn event_helpers_group_by_timestamp() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("battery.db");
        let samples = vec![
            Sample {
                ts: 1.0,
                percentage: Some(50.0),
                capacity_pct: Some(90.0),
                health_pct: Some(95.0),
                energy_now_wh: Some(40.0),
                energy_full_wh: Some(80.0),
                energy_full_design_wh: Some(90.0),
                status: Some("Discharging".to_string()),
                source_path: "BAT0".to_string(),
            },
            Sample {
                ts: 1.0,
                percentage: Some(60.0),
                capacity_pct: Some(91.0),
                health_pct: Some(96.0),
                energy_now_wh: Some(20.0),
                energy_full_wh: Some(40.0),
                energy_full_design_wh: Some(50.0),
                status: Some("Charging".to_string()),
                source_path: "BAT1".to_string(),
            },
            Sample {
                ts: 5.0,
                percentage: Some(75.0),
                capacity_pct: Some(89.0),
                health_pct: Some(94.0),
                energy_now_wh: Some(50.0),
                energy_full_wh: Some(70.0),
                energy_full_design_wh: Some(80.0),
                status: Some("Discharging".to_string()),
                source_path: "BAT0".to_string(),
            },
        ];

        init_db(&db_path).unwrap();
        insert_samples(&db_path, &samples).unwrap();

        assert_eq!(count_events(&db_path, None).unwrap(), 2);
        assert_eq!(count_events(&db_path, Some(2.0)).unwrap(), 1);

        let first_event = fetch_first_event(&db_path).unwrap();
        let latest_event = fetch_latest_event(&db_path).unwrap();
        let recent_events = fetch_recent_events(&db_path, 5).unwrap();

        assert_eq!(first_event.len(), 2);
        assert_eq!(
            first_event
                .iter()
                .map(|s| s.source_path.clone())
                .collect::<std::collections::HashSet<_>>(),
            ["BAT0".to_string(), "BAT1".to_string()]
                .into_iter()
                .collect()
        );
        assert_eq!(latest_event.len(), 1);
        assert_eq!(latest_event[0].ts, 5.0);
        assert_eq!(recent_events.len(), 2);
    }
}
