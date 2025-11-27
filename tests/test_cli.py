def test_last_day_buckets_are_hourly():
    from battery_monitor.cli import _bucket_start
    from datetime import datetime

    # Choose a local time with a non-zero minute to ensure rounding only clears minutes.
    sample_dt = datetime.now().replace(minute=37, second=12, microsecond=123456)
    bucket = _bucket_start(sample_dt.timestamp(), "last_day")

    assert bucket.hour == sample_dt.hour  # no 2-hour grouping
    assert (bucket.minute, bucket.second, bucket.microsecond) == (0, 0, 0)
