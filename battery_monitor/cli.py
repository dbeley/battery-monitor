from __future__ import annotations

import logging
from datetime import datetime
from pathlib import Path
from typing import Iterable, Optional

import typer
from rich.console import Console
from rich.table import Table
from rich import box

from . import db
from .collector import collect_loop, collect_once, resolve_db_path
from .graph import load_series, render_plot

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
    timeframe: str = typer.Option(
        "last_day",
        help="Timeframe: last_3h, last_12h, last_day, last_week, all",
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
    resolved = resolve_db_path(db_path)

    all_samples = list(db.fetch_samples(resolved))
    if not all_samples:
        console.print("No records available; collect data first.")
        raise typer.Exit(code=1)

    samples = load_series(resolved, timeframe)
    if not samples:
        console.print(
            f"No records for {timeframe.replace('_', ' ')}; try a broader timeframe."
        )
        raise typer.Exit(code=1)

    output_path: Optional[Path]
    if graph_path:
        output_path = graph_path
    elif graph:
        output_path = _default_graph_path(timeframe)
    else:
        output_path = None

    if output_path:
        render_plot(samples, show=False, output=output_path)
    summarize(samples, all_samples, timeframe)


def summarize(
    timeframe_samples: Iterable[db.Sample],
    all_samples: list[db.Sample],
    timeframe: str,
) -> None:
    timeframe_samples = list(timeframe_samples)
    last = timeframe_samples[-1]
    timeframe_label = timeframe.replace("_", " ")

    summary = Table(
        title="Database stats",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    summary.add_column("Field")
    summary.add_column("Value")
    summary.add_row("Records (all)", str(len(all_samples)))
    summary.add_row("Records (timeframe)", str(len(timeframe_samples)))
    summary.add_row("First record ts", _format_timestamp(all_samples[0].ts))
    summary.add_row("Latest record ts", _format_timestamp(last.ts))
    summary.add_row("Timeframe window", timeframe_label)
    summary.add_row("Latest status", last.status or "unknown")
    console.print(summary)

    console.print(_recent_table(all_samples))
    console.print(_latest_table(last))
    console.print(_timeframe_report_table(timeframe, timeframe_samples))


def _format_timestamp(ts: float) -> str:
    dt = datetime.fromtimestamp(ts).astimezone()
    return dt.strftime("%Y-%m-%d %H:%M:%S %Z")


def _format_pct(value: Optional[float]) -> str:
    return f"{value:.1f}%" if value is not None else "--"


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

    for sample in reversed(samples[-5:]):
        recent.add_row(
            datetime.fromtimestamp(sample.ts).strftime("%m-%d %H:%M"),
            _format_pct(sample.percentage),
            _format_pct(sample.health_pct),
            sample.status or "unknown",
            Path(sample.source_path).name,
        )
    return recent


def _timeframe_report_table(timeframe: str, samples: list[db.Sample]) -> Table:
    normalized = timeframe.replace("-", "_")
    buckets: dict[datetime, list[db.Sample]] = {}
    for sample in samples:
        bucket_key = _bucket_start(sample.ts, normalized)
        buckets.setdefault(bucket_key, []).append(sample)

    report = Table(
        title=f"{normalized.replace('_', ' ').title()} timeframe report",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    report.add_column("Window", no_wrap=True)
    report.add_column("Records", justify="right")
    report.add_column("Min %", justify="right")
    report.add_column("Avg %", justify="right")
    report.add_column("Max %", justify="right")
    report.add_column("Latest status", no_wrap=True)

    for bucket_start in sorted(buckets):
        window_label = _format_bucket(bucket_start, normalized)
        bucket_samples = buckets[bucket_start]
        pct_values = [s.percentage for s in bucket_samples if s.percentage is not None]
        min_pct, avg_pct, max_pct = _pct_stats(pct_values)
        latest_status = bucket_samples[-1].status or "unknown"
        report.add_row(
            window_label,
            str(len(bucket_samples)),
            min_pct,
            avg_pct,
            max_pct,
            latest_status,
        )
    return report


def _bucket_start(ts: float, timeframe: str) -> datetime:
    dt = datetime.fromtimestamp(ts).astimezone()
    if timeframe == "last_3h":
        minute_bucket = (dt.minute // 15) * 15
        return dt.replace(minute=minute_bucket, second=0, microsecond=0)
    if timeframe == "last_12h":
        return dt.replace(minute=0, second=0, microsecond=0)
    if timeframe == "last_day":
        return dt.replace(minute=0, second=0, microsecond=0)
    if timeframe == "last_week":
        hour_bucket = (dt.hour // 14) * 14
        return dt.replace(hour=hour_bucket, minute=0, second=0, microsecond=0)
    return dt.replace(hour=0, minute=0, second=0, microsecond=0)


def _format_bucket(dt: datetime, timeframe: str) -> str:
    if timeframe == "last_3h":
        return dt.strftime("%m-%d %H:%M")
    if timeframe in {"last_12h", "last_day", "last_week"}:
        return dt.strftime("%m-%d %H:00")
    return dt.strftime("%Y-%m-%d")


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


def main() -> None:
    app()


if __name__ == "__main__":  # pragma: no cover
    main()


def main_collect() -> None:  # pragma: no cover - thin Typer wrapper
    typer.run(collect_command)


def main_report() -> None:  # pragma: no cover - thin Typer wrapper
    typer.run(report_command)
