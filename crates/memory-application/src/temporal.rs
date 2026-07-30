//! Chinese temporal query parsing adapted from the Apache-2.0 `mem0-memLayer` project.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalWindow {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub precision: String,
    pub timezone: String,
    pub source_expression: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemporalError {
    #[error("invalid IANA timezone: {0}")]
    InvalidTimezone(String),
    #[error("local time does not exist in the requested timezone")]
    AmbiguousLocalTime,
    #[error("invalid temporal expression: {0}")]
    InvalidDate(String),
}

pub fn query_has_temporal_intent(query: &str) -> bool {
    const MARKERS: &[&str] = &[
        "刚刚",
        "刚才",
        "近期",
        "最近",
        "过去",
        "前天",
        "昨天",
        "昨日",
        "今天",
        "今日",
        "明天",
        "明日",
        "上周",
        "本周",
        "这周",
        "上个月",
        "上月",
        "本月",
        "这个月",
        "上季度",
        "上一季度",
        "本季度",
        "这个季度",
        "这季度",
        "去年",
        "今年",
    ];
    MARKERS.iter().any(|marker| query.contains(marker))
        || Regex::new(
            r"(?:\d{4}-\d{1,2}-\d{1,2}|\d{4}年|\d{1,2}月\s*\d{1,2}(?:日|号)?|\d{1,4}\s*(?:小时前|天前|小时|天))",
        )
        .expect("built-in temporal intent regex must compile")
        .is_match(query)
}

pub fn parse_temporal_query(
    query: &str,
    reference_time: DateTime<Utc>,
    timezone: &str,
) -> Result<Option<TemporalWindow>, TemporalError> {
    let tz: Tz = timezone
        .parse()
        .map_err(|_| TemporalError::InvalidTimezone(timezone.into()))?;
    let local_now = reference_time.with_timezone(&tz);
    let today = local_now.date_naive();

    let rolling_hours = Regex::new(r"(?:最近|近|过去)\s*(\d{1,4})\s*小时")
        .expect("built-in temporal regex must compile");
    if let Some(captures) = rolling_hours.captures(query) {
        let hours = captures[1]
            .parse::<i64>()
            .map_err(|_| TemporalError::InvalidDate(captures[0].into()))?;
        if !(1..=8_760).contains(&hours) {
            return Err(TemporalError::InvalidDate(captures[0].into()));
        }
        return Ok(Some(TemporalWindow {
            start: reference_time - Duration::hours(hours),
            end: reference_time,
            precision: "rolling_hours".into(),
            timezone: timezone.into(),
            source_expression: captures[0].into(),
        }));
    }

    let rolling_days = Regex::new(r"(?:最近|近|过去)\s*(\d{1,4})\s*天")
        .expect("built-in temporal regex must compile");
    if let Some(captures) = rolling_days.captures(query) {
        let days = captures[1]
            .parse::<i64>()
            .map_err(|_| TemporalError::InvalidDate(captures[0].into()))?;
        if !(1..=3_650).contains(&days) {
            return Err(TemporalError::InvalidDate(captures[0].into()));
        }
        return Ok(Some(TemporalWindow {
            start: reference_time - Duration::days(days),
            end: reference_time,
            precision: "rolling_days".into(),
            timezone: timezone.into(),
            source_expression: captures[0].into(),
        }));
    }
    if query.contains("刚刚") || query.contains("刚才") {
        return Ok(Some(TemporalWindow {
            start: reference_time - Duration::hours(2),
            end: reference_time,
            precision: "rolling_hours".into(),
            timezone: timezone.into(),
            source_expression: "刚刚".into(),
        }));
    }
    if query.contains("近期") || query.trim() == "最近" {
        return Ok(Some(TemporalWindow {
            start: reference_time - Duration::days(30),
            end: reference_time,
            precision: "rolling_days".into(),
            timezone: timezone.into(),
            source_expression: "近期".into(),
        }));
    }

    let explicit_iso = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{1,2})-(?P<day>\d{1,2})")
        .expect("built-in temporal regex must compile");
    let explicit_zh =
        Regex::new(r"(?P<year>\d{4})年\s*(?P<month>\d{1,2})月\s*(?P<day>\d{1,2})(?:日|号)?")
            .expect("built-in temporal regex must compile");
    let month_day = Regex::new(r"(?P<month>\d{1,2})月\s*(?P<day>\d{1,2})(?:日|号)?")
        .expect("built-in temporal regex must compile");
    let relative_days =
        Regex::new(r"(\d{1,4})\s*天前").expect("built-in temporal regex must compile");
    let explicit_quarter =
        Regex::new(r"(?P<year>\d{4})年?\s*第?\s*(?P<quarter>[1-4])\s*(?:季度|季)")
            .expect("built-in temporal regex must compile");
    let explicit_month = Regex::new(r"(?P<year>\d{4})年\s*(?P<month>\d{1,2})月")
        .expect("built-in temporal regex must compile");
    let explicit_year =
        Regex::new(r"(?P<year>\d{4})年").expect("built-in temporal regex must compile");
    let explicit = explicit_iso
        .captures(query)
        .or_else(|| explicit_zh.captures(query));

    if explicit.is_none() {
        if let Some(captures) = explicit_quarter.captures(query) {
            let expression = captures[0].to_owned();
            let year = captures["year"]
                .parse::<i32>()
                .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
            let quarter = captures["quarter"]
                .parse::<u32>()
                .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
            let (start, end) = quarter_bounds(year, quarter, &expression)?;
            return build_window(start, end, "quarter", timezone, &expression, tz).map(Some);
        }
        if query.contains("上季度") || query.contains("上一季度") {
            let current_quarter = (today.month0() / 3) + 1;
            let (year, quarter) = if current_quarter == 1 {
                (today.year() - 1, 4)
            } else {
                (today.year(), current_quarter - 1)
            };
            let (start, end) = quarter_bounds(year, quarter, "上季度")?;
            return build_window(start, end, "quarter", timezone, "上季度", tz).map(Some);
        }
        if query.contains("本季度") || query.contains("这个季度") || query.contains("这季度")
        {
            let quarter = (today.month0() / 3) + 1;
            let (start, end) = quarter_bounds(today.year(), quarter, "本季度")?;
            return build_window(start, end, "quarter", timezone, "本季度", tz).map(Some);
        }
        if let Some(captures) = explicit_month.captures(query) {
            let expression = captures[0].to_owned();
            let year = captures["year"]
                .parse::<i32>()
                .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
            let month = captures["month"]
                .parse::<u32>()
                .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
            let start = NaiveDate::from_ymd_opt(year, month, 1)
                .ok_or_else(|| TemporalError::InvalidDate(expression.clone()))?;
            return build_window(
                start,
                first_day_next_month(start),
                "month",
                timezone,
                &expression,
                tz,
            )
            .map(Some);
        }
        if let Some(captures) = explicit_year.captures(query) {
            let expression = captures[0].to_owned();
            let year = captures["year"]
                .parse::<i32>()
                .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
            let start = NaiveDate::from_ymd_opt(year, 1, 1)
                .ok_or_else(|| TemporalError::InvalidDate(expression.clone()))?;
            let end = NaiveDate::from_ymd_opt(year + 1, 1, 1)
                .ok_or_else(|| TemporalError::InvalidDate(expression.clone()))?;
            return build_window(start, end, "year", timezone, &expression, tz).map(Some);
        }
    }

    let explicit_date = if let Some(captures) = explicit {
        let expression = captures[0].to_owned();
        let year = captures["year"]
            .parse::<i32>()
            .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
        let month = captures["month"]
            .parse::<u32>()
            .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
        let day = captures["day"]
            .parse::<u32>()
            .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
        Some((
            NaiveDate::from_ymd_opt(year, month, day)
                .ok_or_else(|| TemporalError::InvalidDate(expression.clone()))?,
            expression,
        ))
    } else if let Some(captures) = month_day.captures(query) {
        let expression = captures[0].to_owned();
        let month = captures["month"]
            .parse::<u32>()
            .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
        let day = captures["day"]
            .parse::<u32>()
            .map_err(|_| TemporalError::InvalidDate(expression.clone()))?;
        Some((
            NaiveDate::from_ymd_opt(today.year(), month, day)
                .ok_or_else(|| TemporalError::InvalidDate(expression.clone()))?,
            expression,
        ))
    } else {
        None
    };

    let (date, mut precision, expression) = if let Some((date, expression)) = explicit_date {
        (date, "day", expression)
    } else if let Some(captures) = relative_days.captures(query) {
        let days = captures[1]
            .parse::<i64>()
            .map_err(|_| TemporalError::InvalidDate(captures[0].into()))?;
        if !(1..=3_650).contains(&days) {
            return Err(TemporalError::InvalidDate(captures[0].into()));
        }
        (today - Duration::days(days), "day", captures[0].to_owned())
    } else if query.contains("前天") {
        (today - Duration::days(2), "day", "前天".into())
    } else if query.contains("昨天") || query.contains("昨日") {
        (today - Duration::days(1), "day", "昨天".into())
    } else if query.contains("今天") || query.contains("今日") {
        (today, "day", "今天".into())
    } else if query.contains("明天") || query.contains("明日") {
        (today + Duration::days(1), "day", "明天".into())
    } else if query.contains("上周") {
        let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
        return build_window(
            monday - Duration::days(7),
            monday,
            "week",
            timezone,
            "上周",
            tz,
        )
        .map(Some);
    } else if query.contains("本周") || query.contains("这周") {
        let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
        return build_window(
            monday,
            monday + Duration::days(7),
            "week",
            timezone,
            "本周",
            tz,
        )
        .map(Some);
    } else if query.contains("上个月") || query.contains("上月") {
        let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .expect("current year and month must be valid");
        let previous_day = this_month - Duration::days(1);
        let previous_month = NaiveDate::from_ymd_opt(previous_day.year(), previous_day.month(), 1)
            .expect("previous year and month must be valid");
        return build_window(previous_month, this_month, "month", timezone, "上个月", tz).map(Some);
    } else if query.contains("本月") || query.contains("这个月") {
        let this_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
            .expect("current year and month must be valid");
        return build_window(
            this_month,
            first_day_next_month(this_month),
            "month",
            timezone,
            "本月",
            tz,
        )
        .map(Some);
    } else if query.contains("去年") {
        let start =
            NaiveDate::from_ymd_opt(today.year() - 1, 1, 1).expect("previous year must be valid");
        let end = NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("current year must be valid");
        return build_window(start, end, "year", timezone, "去年", tz).map(Some);
    } else if query.contains("今年") {
        let start =
            NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("current year must be valid");
        let end =
            NaiveDate::from_ymd_opt(today.year() + 1, 1, 1).expect("following year must be valid");
        return build_window(start, end, "year", timezone, "今年", tz).map(Some);
    } else {
        return Ok(None);
    };

    let (start_hour, end_hour) = if query.contains("凌晨") {
        precision = "part_of_day";
        (0, 6)
    } else if query.contains("早上") || query.contains("上午") {
        precision = "part_of_day";
        (0, 12)
    } else if query.contains("中午") {
        precision = "part_of_day";
        (11, 14)
    } else if query.contains("下午") {
        precision = "part_of_day";
        (12, 18)
    } else if query.contains("晚上") || query.contains("夜里") {
        precision = "part_of_day";
        (18, 24)
    } else {
        (0, 24)
    };

    let start = local_datetime(tz, date, start_hour)?;
    let end = if end_hour == 24 {
        local_datetime(tz, date + Duration::days(1), 0)?
    } else {
        local_datetime(tz, date, end_hour)?
    };
    Ok(Some(TemporalWindow {
        start,
        end,
        precision: precision.into(),
        timezone: timezone.into(),
        source_expression: expression,
    }))
}

fn first_day_next_month(date: NaiveDate) -> NaiveDate {
    if date.month() == 12 {
        NaiveDate::from_ymd_opt(date.year() + 1, 1, 1).expect("next year must be valid")
    } else {
        NaiveDate::from_ymd_opt(date.year(), date.month() + 1, 1).expect("next month must be valid")
    }
}

fn quarter_bounds(
    year: i32,
    quarter: u32,
    expression: &str,
) -> Result<(NaiveDate, NaiveDate), TemporalError> {
    if !(1..=4).contains(&quarter) {
        return Err(TemporalError::InvalidDate(expression.into()));
    }
    let start_month = (quarter - 1) * 3 + 1;
    let start = NaiveDate::from_ymd_opt(year, start_month, 1)
        .ok_or_else(|| TemporalError::InvalidDate(expression.into()))?;
    let end = if quarter == 4 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, start_month + 3, 1)
    }
    .ok_or_else(|| TemporalError::InvalidDate(expression.into()))?;
    Ok((start, end))
}

fn build_window(
    start: NaiveDate,
    end: NaiveDate,
    precision: &str,
    timezone: &str,
    expression: &str,
    tz: Tz,
) -> Result<TemporalWindow, TemporalError> {
    Ok(TemporalWindow {
        start: local_datetime(tz, start, 0)?,
        end: local_datetime(tz, end, 0)?,
        precision: precision.into(),
        timezone: timezone.into(),
        source_expression: expression.into(),
    })
}

fn local_datetime(tz: Tz, date: NaiveDate, hour: u32) -> Result<DateTime<Utc>, TemporalError> {
    let naive = date
        .and_hms_opt(hour, 0, 0)
        .expect("hour range was validated");
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, _) => Ok(first.with_timezone(&Utc)),
        LocalResult::None => Err(TemporalError::AmbiguousLocalTime),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn reference_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 30, 4, 0, 0).unwrap()
    }

    #[test]
    fn parses_relative_chinese_windows_in_asia_shanghai() {
        let yesterday =
            parse_temporal_query("昨天处理了什么故障", reference_time(), "Asia/Shanghai")
                .unwrap()
                .unwrap();
        assert_eq!(
            yesterday.start,
            Utc.with_ymd_and_hms(2026, 7, 28, 16, 0, 0).unwrap()
        );
        assert_eq!(
            yesterday.end,
            Utc.with_ymd_and_hms(2026, 7, 29, 16, 0, 0).unwrap()
        );
        assert_eq!(yesterday.precision, "day");

        let rolling =
            parse_temporal_query("最近48小时的发布记录", reference_time(), "Asia/Shanghai")
                .unwrap()
                .unwrap();
        assert_eq!(rolling.start, reference_time() - Duration::hours(48));
        assert_eq!(rolling.end, reference_time());
    }

    #[test]
    fn parses_explicit_day_month_quarter_and_year() {
        for (query, start, end, precision) in [
            (
                "查询2025年记录",
                Utc.with_ymd_and_hms(2024, 12, 31, 16, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2025, 12, 31, 16, 0, 0).unwrap(),
                "year",
            ),
            (
                "2026年第2季度发布",
                Utc.with_ymd_and_hms(2026, 3, 31, 16, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 6, 30, 16, 0, 0).unwrap(),
                "quarter",
            ),
            (
                "2026年8月15日验收",
                Utc.with_ymd_and_hms(2026, 8, 14, 16, 0, 0).unwrap(),
                Utc.with_ymd_and_hms(2026, 8, 15, 16, 0, 0).unwrap(),
                "day",
            ),
        ] {
            let parsed = parse_temporal_query(query, reference_time(), "Asia/Shanghai")
                .unwrap()
                .unwrap();
            assert_eq!(parsed.start, start, "{query}");
            assert_eq!(parsed.end, end, "{query}");
            assert_eq!(parsed.precision, precision, "{query}");
        }
    }

    #[test]
    fn rejects_bad_timezone_and_out_of_range_window() {
        assert!(matches!(
            parse_temporal_query("昨天", reference_time(), "Mars/Olympus"),
            Err(TemporalError::InvalidTimezone(_))
        ));
        assert!(matches!(
            parse_temporal_query("过去9999天", reference_time(), "Asia/Shanghai"),
            Err(TemporalError::InvalidDate(_))
        ));
    }
}
