from __future__ import annotations

import logging
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Optional

from . import db
from .timeframe import timeframe_seconds

log = logging.getLogger(__name__)


def load_series(db_path: Path, timeframe: str) -> list[db.Sample]:
    seconds = timeframe_seconds(timeframe)
    since_ts = time.time() - seconds if seconds is not None else None
    return list(db.fetch_samples(db_path, since_ts=since_ts))


def render_plot(
    samples: Iterable[db.Sample], *, show: bool, output: Optional[Path]
) -> None:
    import matplotlib

    # Skip GUI backends when we only need file output; it shortens import time.
    if not show:
        matplotlib.use("Agg", force=True)

    import matplotlib.dates as mdates
    import matplotlib.pyplot as plt

    samples = list(samples)
    if not samples:
        log.warning("No records to plot")
        return

    def _ts_to_num(ts: float) -> float:
        # matplotlib 3.8 dropped mdates.epoch2num; date2num keeps behavior.
        return mdates.date2num(datetime.fromtimestamp(ts, tz=timezone.utc))

    percent_points = [
        (_ts_to_num(s.ts), s.percentage) for s in samples if s.percentage is not None
    ]
    health_points = [
        (_ts_to_num(s.ts), s.health_pct) for s in samples if s.health_pct is not None
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
