from __future__ import annotations

import logging
import os
import time
from pathlib import Path
from typing import Optional

from typer.models import OptionInfo

from . import db
from .sysfs import find_battery_paths, read_battery

log = logging.getLogger(__name__)

DEFAULT_DB_PATH = Path.home() / ".local" / "share" / "battery-monitor" / "battery.db"


def resolve_db_path(db_path: Optional[Path | os.PathLike | str]) -> Path:
    if isinstance(db_path, OptionInfo):
        db_path = db_path.default

    if isinstance(db_path, (str, os.PathLike)):
        db_path = Path(db_path)

    if isinstance(db_path, Path):
        return db_path
    env = os.environ.get("BATTERY_MONITOR_DB")
    if env:
        return Path(env).expanduser()
    return DEFAULT_DB_PATH


def collect_once(
    db_path: Optional[Path] = None, sysfs_root: Optional[Path] = None
) -> int:
    resolved_db = resolve_db_path(db_path)
    db.init_db(resolved_db)

    battery_paths = list(
        find_battery_paths(sysfs_root or Path("/sys/class/power_supply"))
    )
    if not battery_paths:
        log.warning("No batteries found in sysfs")
        return 1

    ts = time.time()
    for path in battery_paths:
        reading = read_battery(path)
        sample = db.create_sample_from_reading(reading, ts=ts)
        db.insert_sample(resolved_db, sample)
        log.info(
            "Recorded sample for %s: percent=%.2f health=%.2f",
            path.name,
            (sample.percentage or 0.0),
            (sample.health_pct or 0.0),
        )
    return 0


def collect_loop(
    interval_seconds: int,
    db_path: Optional[Path] = None,
    sysfs_root: Optional[Path] = None,
) -> None:
    while True:
        collect_once(db_path=db_path, sysfs_root=sysfs_root)
        time.sleep(interval_seconds)
