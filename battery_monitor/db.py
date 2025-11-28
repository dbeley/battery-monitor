from __future__ import annotations

import sqlite3
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator, Optional


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
    insert_samples(db_path, [sample])


def insert_samples(db_path: Path, samples: Iterable[Sample]) -> None:
    with sqlite3.connect(db_path) as conn:
        conn.executemany(
            """
            INSERT INTO samples (
                ts, percentage, capacity_pct, health_pct, energy_now_wh,
                energy_full_wh, energy_full_design_wh, status, source_path
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
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
                )
                for sample in samples
            ),
        )
        conn.commit()


def count_samples(db_path: Path, since_ts: Optional[float] = None) -> int:
    with sqlite3.connect(db_path) as conn:
        if since_ts is None:
            (count,) = conn.execute("SELECT COUNT(*) FROM samples").fetchone()
        else:
            (count,) = conn.execute(
                "SELECT COUNT(*) FROM samples WHERE ts >= ?", (since_ts,)
            ).fetchone()
        return int(count)


def _row_to_sample(row: sqlite3.Row) -> Sample:
    return Sample(
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
            yield _row_to_sample(row)


def fetch_first_sample(db_path: Path) -> Optional[Sample]:
    with sqlite3.connect(db_path) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute("SELECT * FROM samples ORDER BY ts ASC LIMIT 1").fetchone()
        return _row_to_sample(row) if row else None


def fetch_latest_sample(db_path: Path) -> Optional[Sample]:
    with sqlite3.connect(db_path) as conn:
        conn.row_factory = sqlite3.Row
        row = conn.execute("SELECT * FROM samples ORDER BY ts DESC LIMIT 1").fetchone()
        return _row_to_sample(row) if row else None


def fetch_recent_samples(db_path: Path, limit: int = 5) -> list[Sample]:
    with sqlite3.connect(db_path) as conn:
        conn.row_factory = sqlite3.Row
        rows = conn.execute(
            "SELECT * FROM samples ORDER BY ts DESC LIMIT ?", (limit,)
        ).fetchall()
        return [_row_to_sample(row) for row in rows]


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
