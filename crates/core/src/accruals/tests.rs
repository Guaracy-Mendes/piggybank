use super::*;
use bank_platform::domain::Currency;
fn date(s: &str) -> NaiveDate {
    s.parse().unwrap()
}
fn instant(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn money(n: i64) -> Money {
    Money::new(n, Currency::synthetic())
}
pub(super) fn policy() -> PeriodicPolicy {
    PeriodicPolicy {
        id: Uuid::new_v4(),
        code: "DAILY_INTEREST".into(),
        version: 1,
        label: "SYNTHETIC".into(),
        activation: "PREVIEW_ONLY".into(),
        product_version_id: Uuid::new_v4(),
        effective_from: date("2028-02-01"),
        effective_until: date("2028-04-01"),
        calendar: BusinessCalendar {
            id: Uuid::new_v4(),
            code: "SYNTHETIC_UTC".into(),
            version: 1,
            timezone: "UTC".into(),
            coverage_from: date("2028-02-01"),
            coverage_until: date("2028-04-16"),
            business_weekdays: vec![1, 2, 3, 4, 5],
            holidays: vec![date("2028-03-01")],
            cutoff: "17:00:00".into(),
            roll_convention: "FOLLOWING_14_DAYS".into(),
        },
        calculation: Calculation::DailyCreditAccrual {
            annual_basis_points: 365,
            day_count: "ACT_365_FIXED".into(),
            basis: "NONNEGATIVE_CLOSING_BOOKED".into(),
            rounding: "HALF_UP_EACH_DAY".into(),
        },
    }
}
#[test]
fn exact_daily_rounding_large_strings_and_zero() {
    let p = policy();
    for (base, expected) in [
        (0, 0),
        (4999, 0),
        (5000, 1),
        (14999, 1),
        (15000, 2),
        (9_007_199_254_740_993, 900_719_925_474),
        (i64::MAX, 922_337_203_685_478),
    ] {
        let r = p
            .preview(&PreviewInput {
                period_start: date("2028-02-29"),
                closing_booked: Some(money(base)),
                evaluated_at: instant("2028-03-02T17:00:00Z"),
            })
            .unwrap();
        assert_eq!(r.amount.minor_units(), expected);
        assert_eq!(r.exact_numerator, (i128::from(base) * 365).to_string());
        assert_eq!(r.exact_denominator, "3650000");
        assert_eq!(
            serde_json::to_value(r).unwrap()["closingBooked"]["minorUnits"],
            base.to_string()
        );
    }
}
#[test]
fn leap_day_holiday_weekend_and_exact_cutoff() {
    let p = policy();
    let input = PreviewInput {
        period_start: date("2028-02-29"),
        closing_booked: Some(money(5000)),
        evaluated_at: instant("2028-03-02T16:59:59.999999Z"),
    };
    let r = p.preview(&input).unwrap();
    assert_eq!(r.period_end, date("2028-03-01"));
    assert_eq!(r.due_at, instant("2028-03-02T17:00:00Z"));
    assert!(!r.is_due);
    assert!(
        p.preview(&PreviewInput {
            evaluated_at: r.due_at,
            ..input.clone()
        })
        .unwrap()
        .is_due
    );
    let r = p
        .preview(&PreviewInput {
            period_start: date("2028-02-25"),
            ..input
        })
        .unwrap();
    assert_eq!(r.due_at, instant("2028-02-28T17:00:00Z"));
}
#[test]
fn monthly_fee_needs_whole_month_and_never_uses_a_balance() {
    let mut p = policy();
    p.calculation = Calculation::MonthlyFixedFee {
        amount: money(i64::MAX),
    };
    let input = PreviewInput {
        period_start: date("2028-02-01"),
        closing_booked: None,
        evaluated_at: instant("2028-03-02T17:00:00Z"),
    };
    let r = p.preview(&input).unwrap();
    assert_eq!(r.amount.minor_units(), i64::MAX);
    assert_eq!(r.period_end, date("2028-03-01"));
    assert_eq!(r.exact_denominator, "1");
    assert!(
        p.preview(&PreviewInput {
            period_start: date("2028-02-02"),
            ..input.clone()
        })
        .is_err()
    );
    assert!(
        p.preview(&PreviewInput {
            closing_booked: Some(money(0)),
            ..input
        })
        .is_err()
    );
}
#[test]
fn rejects_unsupported_calendar_and_economic_semantics() {
    let p = policy();
    for mutate in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
        let mut x = p.clone();
        match mutate {
            0 => x.calendar.timezone = "America/New_York".into(),
            1 => x.calendar.business_weekdays = vec![5, 1],
            2 => x.calendar.holidays.push(date("2028-03-01")),
            3 => x.calendar.cutoff = "17:00".into(),
            4 => x.calendar.coverage_until = x.effective_until,
            5 => x.activation = "ENABLED".into(),
            6 => x.label = "LIVE".into(),
            7 => x.calendar.business_weekdays.clear(),
            8 => {
                x.calendar.holidays = (0..20)
                    .map(|n| date("2028-02-02") + Duration::days(n))
                    .collect()
            }
            9 => {
                x.calculation = Calculation::DailyCreditAccrual {
                    annual_basis_points: 365,
                    day_count: "ACT_360".into(),
                    basis: "NONNEGATIVE_CLOSING_BOOKED".into(),
                    rounding: "HALF_UP_EACH_DAY".into(),
                }
            }
            11 => x.calendar.cutoff = "23:59:60".into(),
            _ => x.calendar.roll_convention = "PRECEDING".into(),
        };
        assert!(x.validate().is_err(), "mutation {mutate}");
    }
    let input = PreviewInput {
        period_start: date("2028-01-31"),
        closing_booked: Some(money(0)),
        evaluated_at: Utc::now(),
    };
    assert!(p.preview(&input).is_err());
    assert!(
        p.preview(&PreviewInput {
            period_start: p.effective_from,
            closing_booked: Some(money(-1)),
            ..input.clone()
        })
        .is_err()
    );
    assert!(
        p.preview(&PreviewInput {
            period_start: p.effective_from,
            closing_booked: None,
            ..input
        })
        .is_err()
    );
}
#[test]
fn wire_rejects_extra_fields_and_number_money() {
    let p = policy();
    let mut v = serde_json::to_value(&p).unwrap();
    v["calculation"]["extra"] = true.into();
    assert!(serde_json::from_value::<PeriodicPolicy>(v).is_err());
    let mut p = p;
    p.calculation = Calculation::MonthlyFixedFee { amount: money(100) };
    let mut v = serde_json::to_value(p).unwrap();
    v["calculation"]["amount"]["minorUnits"] = 100.into();
    assert!(serde_json::from_value::<PeriodicPolicy>(v).is_err());
}
