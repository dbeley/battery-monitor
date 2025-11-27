from __future__ import annotations

import sqlite3
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, Optional


@dataclass
class Sample:
    ts: float
    percentage: Optional[float]
    capacity_pct: Optional[float]
    health_pct: Optional[float]
    energy_now_wh: Optional[float]
    energy_full_wh: Optional[float]
    energy_full_design_wh: Optional[float]
    status: Optional[str]
    source_path: str


SCHEMA = """
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
"""


def init_db(db_path: Path) -> None:
    db_path.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(db_path) as conn:
        conn.executescript(SCHEMA)


def insert_sample(db_path: Path, sample: Sample) -> None:
    with sqlite3.connect(db_path) as conn:
        conn.execute(
            """
            INSERT INTO samples (
                ts, percentage, capacity_pct, health_pct, energy_now_wh,
                energy_full_wh, energy_full_design_wh, status, source_path
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                sample.ts,
                sample.percentage,
                sample.capacity_pct,
                sample.health_pct,
                sample.energy_now_wh,
                sample.energy_full_wh,
                sample.energy_full_design_wh,
                sample.status,
                sample.source_path,
            ),
        )
        conn.commit()


def fetch_samples(db_path: Path, since_ts: Optional[float] = None) -> Iterator[Sample]:
    with sqlite3.connect(db_path) as conn:
        conn.row_factory = sqlite3.Row
        if since_ts is None:
            rows = conn.execute("SELECT * FROM samples ORDER BY ts").fetchall()
        else:
            rows = conn.execute(
                "SELECT * FROM samples WHERE ts >= ? ORDER BY ts", (since_ts,)
            ).fetchall()
        for row in rows:
            yield Sample(
                ts=row["ts"],
                percentage=row["percentage"],
                capacity_pct=row["capacity_pct"],
                health_pct=row["health_pct"],
                energy_now_wh=row["energy_now_wh"],
                energy_full_wh=row["energy_full_wh"],
                energy_full_design_wh=row["energy_full_design_wh"],
                status=row["status"],
                source_path=row["source_path"],
            )


def create_sample_from_reading(reading, ts: Optional[float] = None) -> Sample:
    now = ts if ts is not None else time.time()
    return Sample(
        ts=now,
        percentage=reading.percentage,
        capacity_pct=reading.capacity_pct,
        health_pct=reading.health_pct,
        energy_now_wh=reading.energy_now_wh,
        energy_full_wh=reading.energy_full_wh,
        energy_full_design_wh=reading.energy_full_design_wh,
        status=reading.status,
        source_path=str(reading.path),
    )
