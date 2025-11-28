from __future__ import annotations

import logging
import math
from datetime import datetime, timedelta
from pathlib import Path
from typing import Iterable, Optional, TYPE_CHECKING

import typer
from rich.console import Console
from rich.table import Table
from rich import box

from . import db
from .aggregate import aggregate_group, aggregate_samples_by_timestamp
from .collector import collect_loop, collect_once, resolve_db_path
from .timeframe import Timeframe, build_timeframe, since_timestamp

if TYPE_CHECKING:
    pass

app = typer.Typer(
    add_completion=False,
    context_settings={"help_option_names": ["-h", "--help"]},
)
console = Console()


def configure_logging(verbose: bool) -> None:
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO, format="%(message)s"
    )


def _sanitize_component(value: str) -> str:
    return "".join(ch if ch.isalnum() or ch in {"-", "_"} else "_" for ch in value)


def _default_graph_path(
    timeframe: str,
    *,
    base_dir: Optional[Path] = None,
    now: Optional[datetime] = None,
) -> Path:
    """Generate an informative, safe graph filename."""
    current = now or datetime.now().astimezone()
    tz_name = _sanitize_component(current.tzname() or "local")
    timeframe_label = timeframe.replace("-", "_")
    timestamp = current.strftime("%Y-%m-%d_%H-%M-%S")
    filename = f"battery_monitor_{timeframe_label}_{timestamp}_{tz_name}.png"
    return (base_dir or Path.cwd()) / filename


@app.command("collect")
def collect_command(
    db_path: Optional[Path] = typer.Option(
        None, help="Path to SQLite database (or set BATTERY_MONITOR_DB)"
    ),
    interval: Optional[int] = typer.Option(
        None, help="Optional interval seconds to loop forever"
    ),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="Enable debug logging"),
) -> None:
    """Collect battery metrics once (or in a loop if interval is set)."""
    configure_logging(verbose)
    if interval:
        collect_loop(interval_seconds=interval, db_path=db_path)
    else:
        raise typer.Exit(code=collect_once(db_path=db_path))


@app.command("report")
def report_command(
    hours: int = typer.Option(
        6,
        "--hours",
        min=1,
        help="Window in hours (used when days/months are zero)",
    ),
    days: int = typer.Option(
        0,
        "--days",
        min=0,
        help="Window in days (overrides hours when non-zero)",
    ),
    months: int = typer.Option(
        0,
        "--months",
        min=0,
        help="Window in months (~30d each; overrides days/hours when non-zero)",
    ),
    all_time: bool = typer.Option(
        False,
        "--all",
        help="Ignore timeframe limits and use the entire history",
    ),
    db_path: Optional[Path] = typer.Option(
        None, help="Path to SQLite database (or set BATTERY_MONITOR_DB)"
    ),
    graph: bool = typer.Option(
        False, "--graph", "-g", help="Save a graph image with an auto-generated name"
    ),
    graph_path: Optional[Path] = typer.Option(
        None,
        "--graph-path",
        help="Custom path for the graph image (png/pdf/etc); overrides --graph name",
    ),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="Enable debug logging"),
) -> None:
    """Render a timeframe report (optionally save a graph image)."""
    configure_logging(verbose)

    timeframe = build_timeframe(
        hours=hours,
        days=days,
        months=months,
        all_time=all_time,
    )
    resolved = resolve_db_path(db_path)

    total_records = db.count_samples(resolved)
    if not total_records:
        console.print("No records available; collect data first.")
        raise typer.Exit(code=1)

    total_events = db.count_events(resolved)
    since_ts = since_timestamp(timeframe)
    raw_samples = list(db.fetch_samples(resolved, since_ts=since_ts))
    samples = aggregate_samples_by_timestamp(raw_samples)
    if not samples:
        console.print(
            f"No records for {timeframe.label.replace('_', ' ')}; try a broader timeframe."
        )
        raise typer.Exit(code=1)

    output_path: Optional[Path]
    if graph_path:
        output_path = graph_path
    elif graph:
        output_path = _default_graph_path(timeframe.label)
    else:
        output_path = None

    if output_path:
        # Import matplotlib lazily only when we actually render a graph.
        from .graph import render_plot

        render_plot(samples, timeframe, show=False, output=output_path)
    first_event = db.fetch_first_event(resolved)
    first_sample = aggregate_group(first_event) if first_event else None
    latest_event = db.fetch_latest_event(resolved)
    latest_sample = aggregate_group(latest_event) if latest_event else samples[-1]
    recent_events = db.fetch_recent_events(resolved)
    recent_samples = [aggregate_group(event) for event in recent_events]
    summarize(
        samples,
        timeframe,
        total_records=total_records,
        total_events=total_events,
        first_sample=first_sample,
        latest_sample=latest_sample,
        recent_samples=recent_samples,
    )


def summarize(
    timeframe_samples: Iterable[db.Sample],
    timeframe: Timeframe,
    *,
    total_records: int,
    total_events: int,
    first_sample: Optional[db.Sample],
    latest_sample: db.Sample,
    recent_samples: list[db.Sample],
) -> None:
    timeframe_samples = list(timeframe_samples)
    last = latest_sample
    timeframe_label = timeframe.label.replace("_", " ")
    avg_consumption_w = _average_consumption_w(timeframe_samples)
    est_runtime_hours = _estimate_runtime_hours(avg_consumption_w, current_sample=last)

    summary = Table(
        title="Database stats",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    summary.add_column("Field")
    summary.add_column("Value")
    summary.add_row("Records (all)", str(total_records))
    summary.add_row("Events (all)", str(total_events))
    summary.add_row("Events (timeframe)", str(len(timeframe_samples)))
    first_ts = first_sample.ts if first_sample else last.ts
    summary.add_row("First record ts", _format_timestamp(first_ts))
    summary.add_row("Latest record ts", _format_timestamp(last.ts))
    summary.add_row("Timeframe window", timeframe_label)
    summary.add_row("Latest status", last.status or "unknown")
    summary.add_row("Avg consumption", _format_power(avg_consumption_w))
    summary.add_row("Est runtime (full)", _format_runtime(est_runtime_hours))
    console.print(summary)

    console.print(_recent_table(recent_samples))
    console.print(_latest_table(last))
    console.print(_timeframe_report_table(timeframe, timeframe_samples))


def _format_timestamp(ts: float) -> str:
    dt = datetime.fromtimestamp(ts).astimezone()
    return dt.strftime("%Y-%m-%d %H:%M:%S %Z")


def _format_pct(value: Optional[float]) -> str:
    return f"{value:.1f}%" if value is not None else "--"


def _format_power(value: Optional[float]) -> str:
    return f"{value:.2f}W" if value is not None else "--"


def _format_runtime(hours: Optional[float]) -> str:
    if hours is None or hours < 0 or math.isinf(hours) or math.isnan(hours):
        return "--"
    minutes = int(hours * 60)
    hrs, mins = divmod(minutes, 60)
    return f"{hrs}h{mins:02d}m"


def _latest_table(sample: db.Sample) -> Table:
    latest = Table(
        title="Latest record",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    latest.add_column("Metric")
    latest.add_column("Value")
    latest.add_row("Charge %", _format_pct(sample.percentage))
    latest.add_row("Health %", _format_pct(sample.health_pct))
    latest.add_row("Capacity %", _format_pct(sample.capacity_pct))
    latest.add_row("Energy now (Wh)", _format_number(sample.energy_now_wh))
    latest.add_row("Energy full (Wh)", _format_number(sample.energy_full_wh))
    latest.add_row("Energy design (Wh)", _format_number(sample.energy_full_design_wh))
    latest.add_row("Source", sample.source_path)
    return latest


def _recent_table(samples: list[db.Sample]) -> Table:
    recent = Table(
        title="Recent records",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    recent.add_column("When", no_wrap=True)
    recent.add_column("Charge", justify="right")
    recent.add_column("Health", justify="right")
    recent.add_column("Status", no_wrap=True)
    recent.add_column("Source")

    for sample in samples:
        recent.add_row(
            datetime.fromtimestamp(sample.ts).strftime("%m-%d %H:%M"),
            _format_pct(sample.percentage),
            _format_pct(sample.health_pct),
            sample.status or "unknown",
            Path(sample.source_path).name,
        )
    return recent


def _timeframe_report_table(timeframe: Timeframe, samples: list[db.Sample]) -> Table:
    bucket_seconds = _bucket_span_seconds(timeframe)
    buckets: dict[datetime, list[db.Sample]] = {}
    for sample in samples:
        bucket_key = _bucket_start(sample.ts, bucket_seconds)
        buckets.setdefault(bucket_key, []).append(sample)

    report = Table(
        title=f"{timeframe.label.replace('_', ' ').title()} timeframe report",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    report.add_column("Window", no_wrap=True)
    report.add_column("Records", justify="right")
    report.add_column("Min %", justify="right")
    report.add_column("Avg %", justify="right")
    report.add_column("Max %", justify="right")
    report.add_column("Avg W", justify="right")
    report.add_column("Latest status", no_wrap=True)

    for bucket_start in sorted(buckets):
        window_label = _format_bucket(bucket_start, bucket_seconds)
        bucket_samples = buckets[bucket_start]
        pct_values = [s.percentage for s in bucket_samples if s.percentage is not None]
        min_pct, avg_pct, max_pct = _pct_stats(pct_values)
        latest_status = bucket_samples[-1].status or "unknown"
        avg_consumption = _average_consumption_w(bucket_samples)
        report.add_row(
            window_label,
            str(len(bucket_samples)),
            min_pct,
            avg_pct,
            max_pct,
            _format_power(avg_consumption),
            latest_status,
        )
    return report


def _bucket_span_seconds(timeframe: Timeframe) -> int:
    """Choose an interval that keeps the table readable for the requested window."""
    window = timeframe.seconds
    if window is None:
        return 7 * 24 * 3600  # weekly buckets for all history
    if window <= 6 * 3600:
        return 20 * 60
    if window <= 24 * 3600:
        return 3600
    if window <= 3 * 24 * 3600:
        return 2 * 3600
    if window <= 7 * 24 * 3600:
        return 6 * 3600
    if window <= 30 * 24 * 3600:
        return 24 * 3600
    if window <= 90 * 24 * 3600:
        return 3 * 24 * 3600
    return 7 * 24 * 3600


def _bucket_start(ts: float, bucket_seconds: int) -> datetime:
    local = datetime.fromtimestamp(ts).astimezone()
    offset = local.utcoffset() or timedelta(0)
    bucket_epoch = int((ts + offset.total_seconds()) // bucket_seconds) * bucket_seconds
    aligned = bucket_epoch - offset.total_seconds()
    return datetime.fromtimestamp(aligned, tz=local.tzinfo)


def _format_bucket(dt: datetime, bucket_seconds: int) -> str:
    if bucket_seconds < 3600:
        return dt.strftime("%m-%d %H:%M")
    if bucket_seconds < 24 * 3600:
        return dt.strftime("%m-%d %H:00")
    days = bucket_seconds // (24 * 3600)
    if days <= 1:
        return dt.strftime("%Y-%m-%d")
    return f"{dt.strftime('%Y-%m-%d')} (+{days}d)"


def _pct_stats(values: list[float]) -> tuple[str, str, str]:
    if not values:
        return ("--", "--", "--")
    return (
        f"{min(values):.1f}%",
        f"{sum(values) / len(values):.1f}%",
        f"{max(values):.1f}%",
    )


def _format_number(value: Optional[float]) -> str:
    return f"{value:.2f}" if value is not None else "--"


def _average_consumption_w(samples: Iterable[db.Sample]) -> Optional[float]:
    # Ignore deltas when samples are far apart (likely machine was off/asleep).
    max_gap_hours = 5 / 60  # only trust 5-minute gaps to ensure the machine was active
    ordered = sorted(
        (s for s in samples if s.energy_now_wh is not None),
        key=lambda sample: sample.ts,
    )
    if len(ordered) < 2:
        return None

    total_delta = 0.0
    total_hours = 0.0
    previous = ordered[0]

    for current in ordered[1:]:
        dt_hours = (current.ts - previous.ts) / 3600
        if dt_hours <= 0:
            previous = current
            continue
        if dt_hours > max_gap_hours:
            previous = current
            continue
        delta_energy = current.energy_now_wh - previous.energy_now_wh
        total_delta += delta_energy
        total_hours += dt_hours
        previous = current

    if total_hours == 0:
        return None

    avg_delta_per_hour = total_delta / total_hours
    return -avg_delta_per_hour  # positive when battery drains


def _estimate_runtime_hours(
    avg_consumption_w: Optional[float], *, current_sample: db.Sample
) -> Optional[float]:
    if avg_consumption_w is None or avg_consumption_w <= 0:
        return None
    capacity_wh = current_sample.energy_full_wh or current_sample.energy_full_design_wh
    if capacity_wh is None or capacity_wh <= 0:
        return None
    return capacity_wh / avg_consumption_w


def main() -> None:
    app()


if __name__ == "__main__":  # pragma: no cover
    main()


def main_collect() -> None:  # pragma: no cover - thin Typer wrapper
    typer.run(collect_command)


def main_report() -> None:  # pragma: no cover - thin Typer wrapper
    typer.run(report_command)
