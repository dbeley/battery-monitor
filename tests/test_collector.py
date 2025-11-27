from pathlib import Path

from typer import Option

from battery_monitor.collector import DEFAULT_DB_PATH, resolve_db_path


def test_resolve_db_path_handles_optioninfo(monkeypatch):
    monkeypatch.delenv("BATTERY_MONITOR_DB", raising=False)

    option_default_none = Option(None)

    assert resolve_db_path(option_default_none) == DEFAULT_DB_PATH


def test_resolve_db_path_handles_optioninfo_with_default(monkeypatch, tmp_path: Path):
    monkeypatch.delenv("BATTERY_MONITOR_DB", raising=False)

    target = tmp_path / "battery.db"
    option_with_default = Option(str(target))

    assert resolve_db_path(option_with_default) == target
