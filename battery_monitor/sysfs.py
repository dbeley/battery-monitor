from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Optional

log = logging.getLogger(__name__)


@dataclass
class BatteryReading:
    path: Path
    capacity_pct: Optional[float]
    percentage: Optional[float]
    energy_now_wh: Optional[float]
    energy_full_wh: Optional[float]
    energy_full_design_wh: Optional[float]
    health_pct: Optional[float]
    status: Optional[str]


def _read_float(path: Path) -> Optional[float]:
    try:
        raw = path.read_text().strip()
    except FileNotFoundError:
        return None
    if not raw:
        return None
    try:
        return float(raw)
    except ValueError:
        log.debug("Non-numeric value in %s: %s", path, raw)
        return None


def _read_str(path: Path) -> Optional[str]:
    try:
        raw = path.read_text().strip()
    except FileNotFoundError:
        return None
    return raw or None


def _wh_from_energy(raw_value: Optional[float]) -> Optional[float]:
    if raw_value is None:
        return None
    # Sysfs energy values are reported in microwatt-hours
    return raw_value / 1_000_000.0


def find_battery_paths(
    sysfs_root: Path = Path("/sys/class/power_supply"),
) -> Iterable[Path]:
    for candidate in sysfs_root.iterdir():
        if candidate.name.startswith("BAT"):
            type_file = candidate / "type"
            try:
                if type_file.read_text().strip().lower() == "battery":
                    yield candidate
            except FileNotFoundError:
                continue


def read_battery(path: Path) -> BatteryReading:
    energy_now = _read_float(path / "energy_now") or _read_float(path / "charge_now")
    energy_full = _read_float(path / "energy_full") or _read_float(path / "charge_full")
    energy_full_design = _read_float(path / "energy_full_design") or _read_float(
        path / "charge_full_design"
    )
    capacity_pct = _read_float(path / "capacity")
    status = _read_str(path / "status")

    energy_now_wh = _wh_from_energy(energy_now)
    energy_full_wh = _wh_from_energy(energy_full)
    energy_full_design_wh = _wh_from_energy(energy_full_design)

    percentage = None
    if energy_now is not None and energy_full:
        try:
            percentage = (energy_now / energy_full) * 100.0
        except ZeroDivisionError:
            percentage = None

    health_pct = None
    if energy_full and energy_full_design:
        try:
            health_pct = (energy_full / energy_full_design) * 100.0
        except ZeroDivisionError:
            health_pct = None

    return BatteryReading(
        path=path,
        capacity_pct=capacity_pct,
        percentage=percentage,
        energy_now_wh=energy_now_wh,
        energy_full_wh=energy_full_wh,
        energy_full_design_wh=energy_full_design_wh,
        health_pct=health_pct,
        status=status,
    )
