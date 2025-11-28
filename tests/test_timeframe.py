from battery_monitor.timeframe import since_timestamp, timeframe_seconds


def test_timeframe_seconds_supports_last_hour():
    assert timeframe_seconds("last_1h") == 3600
    assert timeframe_seconds("last-1h") == 3600


def test_since_timestamp_uses_reference_now():
    now = 1_700_000_000.0
    assert since_timestamp("last_1h", now=now) == now - 3600
