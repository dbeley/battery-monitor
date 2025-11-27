from __future__ import annotations

import logging
from datetime import datetime
from pathlib import Path
from typing import Iterable, Optional

import typer
from rich.console import Console
from rich.table import Table
from rich import box
from rich.panel import Panel

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


@app.command("graph")
def graph_command(
    period: str = typer.Option(
        "last_day", help="Period: last_hour, last_day, last_week, last_month, all"
    ),
    db_path: Optional[Path] = typer.Option(
        None, help="Path to SQLite database (or set BATTERY_MONITOR_DB)"
    ),
    output: Optional[Path] = typer.Option(
        None, help="Optional output image path (png/pdf/etc)"
    ),
    show: bool = typer.Option(False, help="Show the graph interactively"),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="Enable debug logging"),
) -> None:
    """Render a graph for the selected period."""
    configure_logging(verbose)
    resolved = resolve_db_path(db_path)

    all_samples = list(db.fetch_samples(resolved))
    if not all_samples:
        console.print("No samples available; collect data first.")
        raise typer.Exit(code=1)

    samples = load_series(resolved, period)
    if not samples:
        console.print(f"No samples for {period}; try a wider window.")
        raise typer.Exit(code=1)

    render_plot(samples, show=show, output=output)
    summarize(samples, all_samples, period)


def summarize(
    period_samples: Iterable[db.Sample], all_samples: list[db.Sample], period: str
) -> None:
    period_samples = list(period_samples)
    last = period_samples[-1]

    summary = Table(
        title="Battery stats",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    summary.add_column("Field")
    summary.add_column("Value")
    summary.add_row("Records (all)", str(len(all_samples)))
    summary.add_row("Records (period)", str(len(period_samples)))
    summary.add_row("First record", _format_timestamp(all_samples[0].ts))
    summary.add_row("Latest record", _format_timestamp(last.ts))
    summary.add_row("Graphed period", period.replace("_", " "))
    summary.add_row("Latest status", last.status or "unknown")
    console.print(summary)

    console.print(_recent_table(all_samples))
    console.print(_latest_table(last))
    console.print(_period_report_table(period, period_samples))
    console.print(_sparkline_panel(period_samples, period))


def _format_timestamp(ts: float) -> str:
    dt = datetime.fromtimestamp(ts).astimezone()
    return dt.strftime("%Y-%m-%d %H:%M:%S %Z")


def _format_pct(value: Optional[float]) -> str:
    return f"{value:.1f}%" if value is not None else "--"


def _latest_table(sample: db.Sample) -> Table:
    latest = Table(
        title="Latest sample",
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


def _period_report_table(period: str, samples: list[db.Sample]) -> Table:
    buckets: dict[datetime, list[db.Sample]] = {}
    for sample in samples:
        bucket_key = _bucket_start(sample.ts, period)
        buckets.setdefault(bucket_key, []).append(sample)

    report = Table(
        title=f"{period.replace('_', ' ').title()} report",
        show_lines=False,
        box=box.SIMPLE,
        header_style="bold",
    )
    report.add_column("Window", no_wrap=True)
    report.add_column("Samples", justify="right")
    report.add_column("Min %", justify="right")
    report.add_column("Avg %", justify="right")
    report.add_column("Max %", justify="right")
    report.add_column("Latest status", no_wrap=True)

    for bucket_start in sorted(buckets):
        window_label = _format_bucket(bucket_start, period)
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


def _bucket_start(ts: float, period: str) -> datetime:
    dt = datetime.fromtimestamp(ts).astimezone()
    if period in {"last_hour", "last_day"}:
        return dt.replace(minute=0, second=0, microsecond=0)
    return dt.replace(hour=0, minute=0, second=0, microsecond=0)


def _format_bucket(dt: datetime, period: str) -> str:
    if period in {"last_hour", "last_day"}:
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


def _sparkline_panel(samples: list[db.Sample], period: str) -> Panel:
    values = [s.percentage for s in samples if s.percentage is not None]

    if not values:
        return Panel("No percentage data available to plot.", title="CLI graph")

    values = _downsample(values, target=60)
    spark = _sparkline(values)
    return Panel.fit(
        spark,
        title=f"CLI graph ({period.replace('_', ' ')})",
        subtitle="shows % range across window",
        box=box.SIMPLE,
    )


def _downsample(values: list[float], target: int) -> list[float]:
    if len(values) <= target:
        return values
    step = len(values) / target
    return [values[int(i * step)] for i in range(target)]


def _sparkline(values: list[float]) -> str:
    chars = " .:-=+*#%@"
    min_v = min(values)
    max_v = max(values)
    span = max(max_v - min_v, 1e-9)

    def to_char(val: float) -> str:
        idx = int((val - min_v) / span * (len(chars) - 1))
        return chars[min(idx, len(chars) - 1)]

    line = "".join(to_char(v) for v in values)
    return f"{min_v:.0f}% {line} {max_v:.0f}%"


def _format_number(value: Optional[float]) -> str:
    return f"{value:.2f}" if value is not None else "--"


def main() -> None:
    app()


if __name__ == "__main__":  # pragma: no cover
    main()


def main_collect() -> None:  # pragma: no cover - thin Typer wrapper
    typer.run(collect_command)


def main_graph() -> None:  # pragma: no cover - thin Typer wrapper
    typer.run(graph_command)
