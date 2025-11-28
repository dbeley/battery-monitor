def test_last_day_buckets_are_hourly():
    from battery_monitor.cli import _bucket_start
    from datetime import datetime

    # Choose a local time with a non-zero minute to ensure rounding only clears minutes.
    sample_dt = datetime.now().replace(minute=37, second=12, microsecond=123456)
    bucket = _bucket_start(sample_dt.timestamp(), "last_day")

    assert bucket.hour == sample_dt.hour  # no 2-hour grouping
    assert (bucket.minute, bucket.second, bucket.microsecond) == (0, 0, 0)


def test_last_hour_buckets_are_five_minutes():
    from battery_monitor.cli import _bucket_start
    from datetime import datetime

    sample_dt = datetime.now().replace(minute=37, second=42, microsecond=654321)
    bucket = _bucket_start(sample_dt.timestamp(), "last_1h")

    assert bucket.minute == 35  # rounds down to the nearest 5 minutes
    assert (bucket.second, bucket.microsecond) == (0, 0)


def test_default_graph_path_has_timeframe_and_timestamp(tmp_path):
    from datetime import datetime, timezone

    from battery_monitor.cli import _default_graph_path

    now = datetime(2025, 11, 28, 1, 30, 42, tzinfo=timezone.utc)
    path = _default_graph_path("last-3h", base_dir=tmp_path, now=now)

    assert path.parent == tmp_path
    assert path.name == "battery_monitor_last_3h_2025-11-28_01-30-42_UTC.png"
