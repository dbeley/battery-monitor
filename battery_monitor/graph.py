from __future__ import annotations

import logging
import time
from pathlib import Path
from typing import Iterable, Optional

import matplotlib.dates as mdates
import matplotlib.pyplot as plt

from . import db

log = logging.getLogger(__name__)


def _period_seconds(period: str) -> Optional[float]:
    normalized = period.lower().replace("-", "_")
    mapping = {
        "last_hour": 3600,
        "last_day": 86400,
        "last_week": 7 * 86400,
        "last_month": 30 * 86400,
        "all": None,
    }
    if normalized not in mapping:
        raise ValueError(f"Unsupported period: {period}")
    return mapping[normalized]


def load_series(db_path: Path, period: str) -> list[db.Sample]:
    seconds = _period_seconds(period)
    since_ts = time.time() - seconds if seconds is not None else None
    return list(db.fetch_samples(db_path, since_ts=since_ts))


def render_plot(
    samples: Iterable[db.Sample], *, show: bool, output: Optional[Path]
) -> None:
    samples = list(samples)
    if not samples:
        log.warning("No samples to plot")
        return

    percent_points = [
        (mdates.epoch2num(s.ts), s.percentage)
        for s in samples
        if s.percentage is not None
    ]
    health_points = [
        (mdates.epoch2num(s.ts), s.health_pct)
        for s in samples
        if s.health_pct is not None
    ]

    fig, ax = plt.subplots()
    if percent_points:
        times, values = zip(*percent_points)
        ax.plot_date(times, values, "-o", label="Charge %", color="tab:blue")
    if health_points:
        times_h, values_h = zip(*health_points)
        ax.plot_date(times_h, values_h, "-o", label="Health %", color="tab:orange")

    ax.set_xlabel("Time")
    ax.set_ylabel("Percent")
    ax.set_ylim(0, 110)
    ax.xaxis.set_major_formatter(mdates.DateFormatter("%Y-%m-%d %H:%M"))
    fig.autofmt_xdate()
    ax.grid(True, linestyle="--", alpha=0.4)
    ax.legend()
    fig.tight_layout()

    if output:
        output.parent.mkdir(parents=True, exist_ok=True)
        fig.savefig(output)
        log.info("Saved plot to %s", output)
    if show:
        plt.show()
    else:
        plt.close(fig)
