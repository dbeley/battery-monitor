use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use log::{info, warn};

use crate::db::{self, Sample};
use crate::metrics;
use crate::sysfs::{find_battery_paths, read_battery};

pub fn default_db_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    home.join(".local")
        .join("share")
        .join("symmetri")
        .join("metrics.db")
}

pub fn resolve_db_path(db_path: Option<&Path>) -> PathBuf {
    if let Some(path) = db_path {
        return path.to_path_buf();
    }
    if let Ok(env_path) =
        std::env::var("SYMMETRI_DB").or_else(|_| std::env::var("BATTERY_MONITOR_DB"))
    {
        if let Some(stripped) = env_path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(stripped);
            }
        }
        return PathBuf::from(env_path);
    }
    default_db_path()
}

pub fn collect_once(db_path: Option<&Path>, sysfs_root: Option<&Path>) -> Result<i32> {
    let resolved_db = resolve_db_path(db_path);
    let mut conn = db::init_db_connection(&resolved_db)?;

    let root = sysfs_root.unwrap_or_else(|| Path::new("/sys/class/power_supply"));
    let battery_paths = find_battery_paths(root);
    if battery_paths.is_empty() {
        warn!("No batteries found in sysfs; collecting other metrics only");
    }

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let mut samples: Vec<Sample> = Vec::new();
    for path in battery_paths {
        let reading = read_battery(&path);
        samples.push(db::create_sample_from_reading(&reading, Some(ts)));
    }

    let metric_samples = metrics::collect_metrics(ts);
    db::insert_all_samples(&mut conn, &samples, &metric_samples)?;

    if !samples.is_empty() {
        for sample in samples {
            info!(
                "Logged record for {}: percent={:.2} health={:.2}",
                Path::new(&sample.source_path)
                    .file_name()
                    .map(|p| p.to_string_lossy())
                    .unwrap_or_else(|| sample.source_path.clone().into()),
                sample.percentage.unwrap_or(0.0),
                sample.health_pct.unwrap_or(0.0)
            );
        }
    }
    if !metric_samples.is_empty() {
        info!("Logged {} system metric records", metric_samples.len());
    }
    Ok(0)
}

pub fn collect_loop(
    interval_seconds: u64,
    db_path: Option<&Path>,
    sysfs_root: Option<&Path>,
) -> Result<()> {
    loop {
        let _ = collect_once(db_path, sysfs_root)?;
        thread::sleep(Duration::from_secs(interval_seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = env::var(key).ok();
            env::set_var(key, value);
            EnvGuard { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn resolve_db_path_prefers_argument() {
        let _guard = EnvGuard::set("BATTERY_MONITOR_DB", "/tmp/should_not_use.db");
        let provided = PathBuf::from("/tmp/preferred.db");
        let resolved = resolve_db_path(Some(&provided));
        assert_eq!(resolved, provided);
    }

    #[test]
    fn resolve_db_path_expands_home_prefix() {
        let home = dirs::home_dir().expect("home directory not found");
        let _guard = EnvGuard::set("SYMMETRI_DB", "~/custom/battery.db");
        let resolved = resolve_db_path(None);
        assert_eq!(resolved, home.join("custom").join("battery.db"));
    }

    #[test]
    fn resolve_db_path_uses_env_verbatim() {
        let _guard = EnvGuard::set("SYMMETRI_DB", "/tmp/from_env.db");
        let resolved = resolve_db_path(None);
        assert_eq!(resolved, PathBuf::from("/tmp/from_env.db"));
    }

    #[test]
    fn resolve_db_path_accepts_legacy_env() {
        let _guard = EnvGuard::set("BATTERY_MONITOR_DB", "/tmp/legacy.db");
        let resolved = resolve_db_path(None);
        assert_eq!(resolved, PathBuf::from("/tmp/legacy.db"));
    }
}
