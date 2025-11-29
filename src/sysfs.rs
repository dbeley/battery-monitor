use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct BatteryReading {
    pub path: PathBuf,
    pub capacity_pct: Option<f64>,
    pub percentage: Option<f64>,
    pub energy_now_wh: Option<f64>,
    pub energy_full_wh: Option<f64>,
    pub energy_full_design_wh: Option<f64>,
    pub health_pct: Option<f64>,
    pub status: Option<String>,
}

fn parse_uevent(path: &Path) -> HashMap<String, String> {
    let mut data = HashMap::new();
    let content = fs::read_to_string(path.join("uevent")).unwrap_or_default();
    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            data.insert(key.to_string(), value.to_string());
        }
    }
    data
}

fn read_float(path: &Path) -> Option<f64> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

fn float_from_uevent(uevent: &HashMap<String, String>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(raw) = uevent.get(*key) {
            if let Ok(value) = raw.parse::<f64>() {
                return Some(value);
            }
        }
    }
    None
}

fn read_str(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn wh_from_energy(raw_value: Option<f64>) -> Option<f64> {
    raw_value.map(|v| v / 1_000_000.0)
}

fn energy_wh_from_charge(charge_uah: Option<f64>, voltage_uv: Option<f64>) -> Option<f64> {
    match (charge_uah, voltage_uv) {
        (Some(charge), Some(voltage)) => Some((charge * voltage) / 1_000_000_000_000.0),
        _ => None,
    }
}

fn read_voltage(path: &Path, uevent: &HashMap<String, String>) -> Option<f64> {
    float_from_uevent(
        uevent,
        &[
            "POWER_SUPPLY_VOLTAGE_NOW",
            "POWER_SUPPLY_VOLTAGE_MIN_DESIGN",
            "POWER_SUPPLY_VOLTAGE_MAX_DESIGN",
        ],
    )
    .or_else(|| {
        for name in ["voltage_now", "voltage_min_design", "voltage_max_design"] {
            if let Some(value) = read_float(&path.join(name)) {
                return Some(value);
            }
        }
        None
    })
}

pub fn find_battery_paths(sysfs_root: &Path) -> Vec<PathBuf> {
    let mut batteries = Vec::new();
    let entries = match fs::read_dir(sysfs_root) {
        Ok(entries) => entries,
        Err(_) => return batteries,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|p| p.to_str())
            .map(|name| name.starts_with("BAT"))
            .unwrap_or(false)
        {
            continue;
        }
        let type_file = path.join("type");
        if let Ok(raw) = fs::read_to_string(&type_file) {
            if raw.trim().eq_ignore_ascii_case("battery") {
                batteries.push(path);
            }
        }
    }
    batteries
}

pub fn read_battery(path: &Path) -> BatteryReading {
    let uevent = parse_uevent(path);

    let mut energy_now_raw = float_from_uevent(&uevent, &["POWER_SUPPLY_ENERGY_NOW"]);
    if energy_now_raw.is_none() {
        energy_now_raw = read_float(&path.join("energy_now"));
    }

    let mut energy_full_raw = float_from_uevent(&uevent, &["POWER_SUPPLY_ENERGY_FULL"]);
    if energy_full_raw.is_none() {
        energy_full_raw = read_float(&path.join("energy_full"));
    }

    let mut energy_full_design_raw =
        float_from_uevent(&uevent, &["POWER_SUPPLY_ENERGY_FULL_DESIGN"]);
    if energy_full_design_raw.is_none() {
        energy_full_design_raw = read_float(&path.join("energy_full_design"));
    }

    let mut charge_now = float_from_uevent(&uevent, &["POWER_SUPPLY_CHARGE_NOW"]);
    if charge_now.is_none() {
        charge_now = read_float(&path.join("charge_now"));
    }

    let mut charge_full = float_from_uevent(&uevent, &["POWER_SUPPLY_CHARGE_FULL"]);
    if charge_full.is_none() {
        charge_full = read_float(&path.join("charge_full"));
    }

    let mut charge_full_design = float_from_uevent(&uevent, &["POWER_SUPPLY_CHARGE_FULL_DESIGN"]);
    if charge_full_design.is_none() {
        charge_full_design = read_float(&path.join("charge_full_design"));
    }

    let mut capacity_pct = float_from_uevent(&uevent, &["POWER_SUPPLY_CAPACITY"]);
    if capacity_pct.is_none() {
        capacity_pct = read_float(&path.join("capacity"));
    }

    let status = uevent
        .get("POWER_SUPPLY_STATUS")
        .cloned()
        .or_else(|| read_str(&path.join("status")));
    let voltage = read_voltage(path, &uevent);

    let mut energy_now_wh = wh_from_energy(energy_now_raw);
    let mut energy_full_wh = wh_from_energy(energy_full_raw);
    let mut energy_full_design_wh = wh_from_energy(energy_full_design_raw);

    if energy_now_wh.is_none() {
        energy_now_wh = energy_wh_from_charge(charge_now, voltage);
    }
    if energy_full_wh.is_none() {
        energy_full_wh = energy_wh_from_charge(charge_full, voltage);
    }
    if energy_full_design_wh.is_none() {
        energy_full_design_wh = energy_wh_from_charge(charge_full_design, voltage);
    }

    let mut percentage = None;
    if let (Some(now), Some(full)) = (energy_now_wh, energy_full_wh) {
        if full != 0.0 {
            percentage = Some((now / full) * 100.0);
        }
    }

    let mut health_pct = None;
    if let (Some(full), Some(design)) = (energy_full_wh, energy_full_design_wh) {
        if design != 0.0 {
            health_pct = Some((full / design) * 100.0);
        }
    }

    BatteryReading {
        path: path.to_path_buf(),
        capacity_pct,
        percentage,
        energy_now_wh,
        energy_full_wh,
        energy_full_design_wh,
        health_pct,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, value: &str) {
        fs::write(path, value).unwrap();
    }

    #[test]
    fn find_battery_paths_filters_to_bat_devices() {
        let tmp = tempfile::tempdir().unwrap();
        let bat0 = tmp.path().join("BAT0");
        fs::create_dir(&bat0).unwrap();
        write(&bat0.join("type"), "Battery\n");

        let ac = tmp.path().join("AC");
        fs::create_dir(&ac).unwrap();
        write(&ac.join("type"), "Mains\n");

        let paths = find_battery_paths(tmp.path());
        assert_eq!(paths, vec![bat0]);
    }

    #[test]
    fn read_battery_uses_energy_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let bat = tmp.path().join("BAT1");
        fs::create_dir(&bat).unwrap();
        write(&bat.join("type"), "Battery\n");
        write(&bat.join("energy_now"), "40000000\n");
        write(&bat.join("energy_full"), "80000000\n");
        write(&bat.join("energy_full_design"), "90000000\n");
        write(&bat.join("capacity"), "95\n");
        write(&bat.join("status"), "Discharging\n");

        let reading = read_battery(&bat);
        assert_eq!(reading.energy_now_wh, Some(40.0));
        assert_eq!(reading.energy_full_wh, Some(80.0));
        assert_eq!(reading.energy_full_design_wh, Some(90.0));
        assert!((reading.percentage.unwrap() - 50.0).abs() < 1e-6);
        assert!((reading.health_pct.unwrap() - (80.0 / 90.0 * 100.0)).abs() < 1e-6);
        assert_eq!(reading.capacity_pct, Some(95.0));
        assert_eq!(reading.status.as_deref(), Some("Discharging"));
    }

    #[test]
    fn read_battery_uses_charge_and_voltage_when_energy_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let bat = tmp.path().join("BAT0");
        fs::create_dir(&bat).unwrap();
        write(&bat.join("type"), "Battery\n");
        write(&bat.join("charge_now"), "2000000\n");
        write(&bat.join("charge_full"), "4000000\n");
        write(&bat.join("charge_full_design"), "4500000\n");
        write(&bat.join("voltage_min_design"), "11000000\n");
        write(&bat.join("capacity"), "90\n");
        write(&bat.join("status"), "Charging\n");

        let reading = read_battery(&bat);
        assert_eq!(reading.energy_now_wh, Some(22.0));
        assert_eq!(reading.energy_full_wh, Some(44.0));
        assert_eq!(reading.energy_full_design_wh, Some(49.5));
        assert!((reading.percentage.unwrap() - 50.0).abs() < 1e-6);
        assert!((reading.health_pct.unwrap() - (44.0 / 49.5 * 100.0)).abs() < 1e-6);
        assert_eq!(reading.capacity_pct, Some(90.0));
        assert_eq!(reading.status.as_deref(), Some("Charging"));
    }

    #[test]
    fn read_battery_prefers_uevent_values() {
        let tmp = tempfile::tempdir().unwrap();
        let bat = tmp.path().join("BAT2");
        fs::create_dir(&bat).unwrap();
        let content = [
            "POWER_SUPPLY_ENERGY_NOW=30000000",
            "POWER_SUPPLY_ENERGY_FULL=60000000",
            "POWER_SUPPLY_ENERGY_FULL_DESIGN=80000000",
            "POWER_SUPPLY_CAPACITY=85",
            "POWER_SUPPLY_STATUS=Discharging",
        ]
        .join("\n");
        write(&bat.join("uevent"), &content);

        let reading = read_battery(&bat);
        assert_eq!(reading.energy_now_wh, Some(30.0));
        assert_eq!(reading.energy_full_wh, Some(60.0));
        assert_eq!(reading.energy_full_design_wh, Some(80.0));
        assert!((reading.percentage.unwrap() - 50.0).abs() < 1e-6);
        assert!((reading.health_pct.unwrap() - (60.0 / 80.0 * 100.0)).abs() < 1e-6);
        assert_eq!(reading.capacity_pct, Some(85.0));
        assert_eq!(reading.status.as_deref(), Some("Discharging"));
    }
}
