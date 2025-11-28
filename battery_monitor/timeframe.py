from __future__ import annotations

import time
from typing import Optional


def normalize_timeframe(timeframe: str) -> str:
    return timeframe.lower().replace("-", "_")


def timeframe_seconds(timeframe: str) -> Optional[float]:
    normalized = normalize_timeframe(timeframe)
    mapping = {
        "last_1h": 3600,
        "last_3h": 3 * 3600,
        "last_12h": 12 * 3600,
        "last_day": 86400,
        "last_week": 7 * 86400,
        "all": None,
    }
    if normalized not in mapping:
        raise ValueError(f"Unsupported timeframe: {timeframe}")
    return mapping[normalized]


def since_timestamp(timeframe: str, *, now: Optional[float] = None) -> Optional[float]:
    seconds = timeframe_seconds(timeframe)
    if seconds is None:
        return None
    reference = now if now is not None else time.time()
    return reference - seconds
