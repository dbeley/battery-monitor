use std::time::SystemTime;

const SECONDS_PER_HOUR: u64 = 3600;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;
const APPROX_DAYS_PER_MONTH: u64 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct Timeframe {
    pub label: String,
    pub seconds: Option<f64>,
    pub hours: u64,
    pub days: u64,
    pub months: u64,
}

impl Timeframe {
    pub fn since_timestamp(&self, now: Option<SystemTime>) -> Option<f64> {
        let seconds = self.seconds?;
        let reference = now.unwrap_or_else(SystemTime::now);
        let reference_secs = reference
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        Some(reference_secs - seconds)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TimeframeError {
    #[error("hours must be at least 1 when days and months are zero")]
    InvalidHours,
    #[error("{0} must be zero or greater")]
    NegativeValue(&'static str),
}

fn validate_non_negative(value: i64, name: &'static str) -> Result<(), TimeframeError> {
    if value < 0 {
        return Err(TimeframeError::NegativeValue(name));
    }
    Ok(())
}

fn plural_suffix(value: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if value == 1 {
        singular
    } else {
        plural
    }
}

pub fn build_timeframe(
    hours: i64,
    days: i64,
    months: i64,
    all_time: bool,
) -> Result<Timeframe, TimeframeError> {
    validate_non_negative(hours, "hours")?;
    validate_non_negative(days, "days")?;
    validate_non_negative(months, "months")?;

    if all_time {
        return Ok(Timeframe {
            label: "all".to_string(),
            seconds: None,
            hours: 0,
            days: 0,
            months: 0,
        });
    }

    if months > 0 {
        let months_u = months as u64;
        let seconds = months_u * APPROX_DAYS_PER_MONTH * SECONDS_PER_DAY;
        let label = format!(
            "last_{}_{}",
            months_u,
            plural_suffix(months_u, "month", "months")
        );
        return Ok(Timeframe {
            label,
            seconds: Some(seconds as f64),
            hours: 0,
            days: 0,
            months: months_u,
        });
    }

    if days > 0 {
        let days_u = days as u64;
        let seconds = days_u * SECONDS_PER_DAY;
        let label = format!("last_{}_{}", days_u, plural_suffix(days_u, "day", "days"));
        return Ok(Timeframe {
            label,
            seconds: Some(seconds as f64),
            hours: 0,
            days: days_u,
            months: 0,
        });
    }

    if hours == 0 {
        return Err(TimeframeError::InvalidHours);
    }

    let hours_u = hours as u64;
    let seconds = hours_u * SECONDS_PER_HOUR;
    let label = format!(
        "last_{}_{}",
        hours_u,
        plural_suffix(hours_u, "hour", "hours")
    );
    Ok(Timeframe {
        label,
        seconds: Some(seconds as f64),
        hours: hours_u,
        days: 0,
        months: 0,
    })
}

pub fn timeframe_seconds(timeframe: &Timeframe) -> Option<f64> {
    timeframe.seconds
}

pub fn since_timestamp(timeframe: &Timeframe, now: Option<SystemTime>) -> Option<f64> {
    timeframe.since_timestamp(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn default_timeframe_is_last_six_hours() {
        let timeframe = build_timeframe(6, 0, 0, false).unwrap();
        assert_eq!(timeframe.hours, 6);
        assert_eq!(timeframe.seconds, Some(6.0 * 3600.0));
        assert_eq!(timeframe.label, "last_6_hours");
    }

    #[test]
    fn days_and_months_take_precedence_over_hours() {
        let timeframe_days = build_timeframe(2, 1, 0, false).unwrap();
        let timeframe_months = build_timeframe(2, 2, 1, false).unwrap();

        assert_eq!(timeframe_days.days, 1);
        assert_eq!(timeframe_days.seconds, Some(24.0 * 3600.0));
        assert_eq!(timeframe_days.label, "last_1_day");

        assert_eq!(timeframe_months.months, 1);
        assert_eq!(timeframe_months.seconds, Some((30 * 24 * 3600) as f64));
        assert_eq!(timeframe_months.label, "last_1_month");
    }

    #[test]
    fn since_timestamp_uses_reference() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let timeframe = build_timeframe(1, 0, 0, false).unwrap();
        let since = since_timestamp(&timeframe, Some(now)).unwrap();
        assert_eq!(since, 1_700_000_000.0 - 3600.0);
    }

    #[test]
    fn since_timestamp_allows_unbounded() {
        let timeframe = build_timeframe(6, 0, 0, true).unwrap();
        assert!(since_timestamp(&timeframe, None).is_none());
        assert_eq!(timeframe.label, "all");
    }

    #[test]
    fn invalid_inputs_raise() {
        assert!(matches!(
            build_timeframe(0, 0, 0, false),
            Err(TimeframeError::InvalidHours)
        ));
        assert!(matches!(
            build_timeframe(1, -1, 0, false),
            Err(TimeframeError::NegativeValue(_))
        ));
    }
}
