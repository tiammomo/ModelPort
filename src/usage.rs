use crate::pricing::{self, TokenUsageBreakdown};

#[derive(Debug, Clone, Copy, Default)]
pub struct UsageEstimate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_estimate: f64,
}

pub(crate) const DAY_MS: u64 = 24 * 60 * 60 * 1_000;

pub(crate) fn quota_increment(quota_type: &str, estimate: UsageEstimate) -> f64 {
    match quota_type {
        "requests" => 1.0,
        "tokens" => estimate
            .input_tokens
            .saturating_add(estimate.output_tokens)
            .saturating_add(estimate.cache_write_tokens)
            .saturating_add(estimate.cache_read_tokens) as f64,
        "cost" => estimate.cost_estimate,
        _ => 0.0,
    }
}

pub(crate) trait UsageCostRecord {
    fn timestamp_ms(&self) -> u64;
    fn api_key_id(&self) -> Option<&str>;
    fn team_id(&self) -> Option<&str>;
    fn resolved_model(&self) -> &str;
    fn token_usage(&self) -> TokenUsageBreakdown;
    fn cost_estimate(&self) -> f64;
}

#[cfg(test)]
pub(crate) fn usage_cost_for_api_key<T: UsageCostRecord>(
    usage: &[T],
    api_key_id: &str,
    since: Option<u64>,
) -> f64 {
    usage
        .iter()
        .filter(|record| record.api_key_id() == Some(api_key_id))
        .filter(|record| since.is_none_or(|since| record.timestamp_ms() >= since))
        .map(usage_record_cost)
        .map(|cost| cost.max(0.0))
        .sum()
}

pub(crate) fn usage_cost_for_team<T: UsageCostRecord>(
    usage: &[T],
    team_id: &str,
    since: Option<u64>,
) -> f64 {
    usage
        .iter()
        .filter(|record| record.team_id() == Some(team_id))
        .filter(|record| since.is_none_or(|since| record.timestamp_ms() >= since))
        .map(usage_record_cost)
        .map(|cost| cost.max(0.0))
        .sum()
}

pub(crate) fn usage_record_cost(record: &impl UsageCostRecord) -> f64 {
    let usage = record.token_usage();
    let has_token_breakdown = usage
        .input_tokens
        .saturating_add(usage.output_tokens)
        .saturating_add(usage.cache_write_tokens)
        .saturating_add(usage.cache_read_tokens)
        > 0;
    if !has_token_breakdown {
        return record.cost_estimate();
    }

    pricing::cost_for_model(record.resolved_model(), usage)
}

pub(crate) fn current_period(period: &str, now: u64) -> (u64, u64) {
    match period {
        "daily" => {
            let start = day_start(now);
            (start, start.saturating_add(DAY_MS))
        }
        "weekly" => {
            // UTC calendar week, Monday 00:00 through the next Monday.
            let days_since_epoch = now / DAY_MS;
            let weekday_from_monday = days_since_epoch.saturating_add(3) % 7;
            let start = days_since_epoch
                .saturating_sub(weekday_from_monday)
                .saturating_mul(DAY_MS);
            (start, start.saturating_add(DAY_MS * 7))
        }
        "monthly" => {
            // UTC calendar month; use civil-date arithmetic so leap years and
            // 28/29/30/31-day months reset on the first day as users expect.
            let days_since_epoch = i64::try_from(now / DAY_MS).unwrap_or(i64::MAX);
            let (year, month, _) = civil_from_days(days_since_epoch);
            let (next_year, next_month) = if month == 12 {
                (year.saturating_add(1), 1)
            } else {
                (year, month + 1)
            };
            let start = millis_from_civil(year, month, 1);
            let end = millis_from_civil(next_year, next_month, 1);
            (start, end)
        }
        _ => (now, now.saturating_add(DAY_MS)),
    }
}

fn millis_from_civil(year: i64, month: u32, day: u32) -> u64 {
    u64::try_from(days_from_civil(year, month, day))
        .unwrap_or(0)
        .saturating_mul(DAY_MS)
}

// Gregorian civil-date conversions adapted from Howard Hinnant's public-domain
// algorithms. Inputs and outputs are days relative to 1970-01-01 UTC.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch.saturating_add(719_468);
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(crate) fn day_start(now: u64) -> u64 {
    (now / DAY_MS) * DAY_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestUsageRecord {
        timestamp_ms: u64,
        api_key_id: Option<String>,
        team_id: Option<String>,
        resolved_model: String,
        token_usage: TokenUsageBreakdown,
        cost_estimate: f64,
    }

    impl UsageCostRecord for TestUsageRecord {
        fn timestamp_ms(&self) -> u64 {
            self.timestamp_ms
        }

        fn api_key_id(&self) -> Option<&str> {
            self.api_key_id.as_deref()
        }

        fn team_id(&self) -> Option<&str> {
            self.team_id.as_deref()
        }

        fn resolved_model(&self) -> &str {
            &self.resolved_model
        }

        fn token_usage(&self) -> TokenUsageBreakdown {
            self.token_usage
        }

        fn cost_estimate(&self) -> f64 {
            self.cost_estimate
        }
    }

    #[test]
    fn quota_increment_matches_quota_type() {
        let estimate = UsageEstimate {
            input_tokens: 10,
            output_tokens: 20,
            cache_write_tokens: 30,
            cache_read_tokens: 40,
            cost_estimate: 0.125,
        };

        assert_eq!(quota_increment("requests", estimate), 1.0);
        assert_eq!(quota_increment("tokens", estimate), 100.0);
        assert_eq!(quota_increment("cost", estimate), 0.125);
        assert_eq!(quota_increment("unknown", estimate), 0.0);
    }

    #[test]
    fn quota_periods_use_utc_calendar_boundaries() {
        assert_eq!(
            current_period("weekly", 1_720_569_600_000),
            (1_720_396_800_000, 1_721_001_600_000)
        );
        assert_eq!(
            current_period("monthly", 1_707_955_200_000),
            (1_706_745_600_000, 1_709_251_200_000)
        );
    }

    #[test]
    fn period_windows_are_stable_and_monotonic() {
        let now = DAY_MS * 42 + 1234;

        assert_eq!(current_period("daily", now), (DAY_MS * 42, DAY_MS * 43));
        // 1970-02-12 is a Thursday: its UTC calendar week starts on
        // Monday 1970-02-09 (epoch day 39), and February spans days 31..59.
        assert_eq!(current_period("weekly", now), (DAY_MS * 39, DAY_MS * 46));
        assert_eq!(current_period("monthly", now), (DAY_MS * 31, DAY_MS * 59));
        assert_eq!(current_period("custom", now), (now, now + DAY_MS));
    }

    #[test]
    fn usage_record_cost_prefers_token_pricing_when_tokens_exist() {
        let record = TestUsageRecord {
            timestamp_ms: 1,
            api_key_id: Some("key_a".to_owned()),
            team_id: Some("team_a".to_owned()),
            resolved_model: "deepseek-v4-flash".to_owned(),
            token_usage: TokenUsageBreakdown {
                input_tokens: 1_000,
                output_tokens: 2_000,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
            },
            cost_estimate: 999.0,
        };

        assert_eq!(
            usage_record_cost(&record),
            pricing::cost_for_model("deepseek-v4-flash", record.token_usage)
        );
    }

    #[test]
    fn usage_record_cost_falls_back_to_stored_estimate_without_tokens() {
        let record = TestUsageRecord {
            timestamp_ms: 1,
            api_key_id: Some("key_a".to_owned()),
            team_id: Some("team_a".to_owned()),
            resolved_model: "deepseek-v4-flash".to_owned(),
            token_usage: TokenUsageBreakdown::default(),
            cost_estimate: 0.42,
        };

        assert_eq!(usage_record_cost(&record), 0.42);
    }

    #[test]
    fn usage_cost_aggregation_filters_by_owner_and_time() {
        let records = vec![
            TestUsageRecord {
                timestamp_ms: 100,
                api_key_id: Some("key_a".to_owned()),
                team_id: Some("team_a".to_owned()),
                resolved_model: "unknown".to_owned(),
                token_usage: TokenUsageBreakdown::default(),
                cost_estimate: 1.0,
            },
            TestUsageRecord {
                timestamp_ms: 200,
                api_key_id: Some("key_a".to_owned()),
                team_id: Some("team_b".to_owned()),
                resolved_model: "unknown".to_owned(),
                token_usage: TokenUsageBreakdown::default(),
                cost_estimate: 2.0,
            },
            TestUsageRecord {
                timestamp_ms: 300,
                api_key_id: Some("key_b".to_owned()),
                team_id: Some("team_a".to_owned()),
                resolved_model: "unknown".to_owned(),
                token_usage: TokenUsageBreakdown::default(),
                cost_estimate: -10.0,
            },
        ];

        assert_eq!(usage_cost_for_api_key(&records, "key_a", Some(150)), 2.0);
        assert_eq!(usage_cost_for_team(&records, "team_a", None), 1.0);
    }
}
