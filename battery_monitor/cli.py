from __future__ import annotations

import logging
from pathlib import Path
from typing import Optional

import typer
from rich.console import Console
from rich.table import Table

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
    samples = load_series(resolved, period)
    if not samples:
        console.print("No samples available; collect data first.")
        raise typer.Exit(code=1)
    render_plot(samples, show=show, output=output)
    summarize(samples)


def summarize(samples) -> None:
    table = Table(title="Battery stats", show_lines=False)
    table.add_column("Field")
    table.add_column("Value")
    last = samples[-1]
    table.add_row("Samples", str(len(samples)))
    table.add_row(
        "Last %",
        f"{(last.percentage or 0):.2f}" if last.percentage is not None else "n/a",
    )
    if last.health_pct is not None:
        table.add_row("Health %", f"{last.health_pct:.2f}")
    if last.capacity_pct is not None:
        table.add_row("Sysfs capacity", f"{last.capacity_pct:.2f}")
    table.add_row("Status", last.status or "unknown")
    console.print(table)


def main() -> None:
    app()


if __name__ == "__main__":  # pragma: no cover
    main()
