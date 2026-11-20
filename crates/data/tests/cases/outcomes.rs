use crate::analytics::{close, world};
use bank_data::outcomes::KpiWindow;
use chrono::{Duration, Utc};
use serde_json::Value;

fn kpi<'a>(snapshot: &'a Value, code: &str) -> &'a Value {
    snapshot["kpis"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["code"] == code)
        .unwrap()
}

#[tokio::test]
async fn kpi_snapshots_state_numerators_denominators_and_exclusions_and_are_labelled_local()
-> anyhow::Result<()> {
    let w = world().await?;
    let window = KpiWindow {
        from: Utc::now() - Duration::days(1),
        to: Utc::now() + Duration::hours(1),
    };
    // Only reporting roles may compute KPIs; windows are bounded.
    let denied = w.wh.kpi_snapshot(&w.controls, &w.analyst, &window).await;
    assert!(format!("{:#}", denied.unwrap_err()).contains("role"));
    let wide = KpiWindow {
        from: Utc::now() - Duration::days(120),
        to: Utc::now(),
    };
    assert!(
        format!(
            "{:#}",
            w.wh.kpi_snapshot(&w.controls, &w.steward, &wide)
                .await
                .unwrap_err()
        )
        .contains("at most 93 days")
    );

    let s = w.wh.kpi_snapshot(&w.controls, &w.steward, &window).await?;
    assert_eq!(s["definitionsVersion"], "kpi-v1");
    assert_eq!(s["evidence"], "LOCAL_SYNTHETIC_NOT_PRODUCTION_OUTCOMES");
    assert_eq!(s["targetsRatification"], "PENDING_DEC-07_DEC-12");
    assert_eq!(s["kpis"].as_array().unwrap().len(), 8);
    // Two accounts opened and activated without remediation.
    let k3 = kpi(&s, "KPI-03");
    assert_eq!(
        (k3["numerator"].as_i64(), k3["denominator"].as_i64()),
        (Some(2), Some(2))
    );
    assert_eq!(k3["value"], "1.0000");
    assert_eq!(k3["meetsTarget"], true);
    // Funding and transfer batches balance and are not duplicated.
    let k5 = kpi(&s, "KPI-05");
    assert_eq!(k5["value"], "0");
    assert_eq!(k5["meetsTarget"], true);
    assert!(k5["denominator"].as_i64().unwrap() >= 2);
    // No payment intents or applications in this fixture: reported as having no denominator, never as success.
    let k4 = kpi(&s, "KPI-04");
    assert_eq!(k4["status"], "NO_DENOMINATOR");
    assert!(k4["value"].is_null() && k4["meetsTarget"].is_null());
    assert_eq!(k4["slaBasis"], "LOCAL_ASSUMED_PENDING_DEC-04");
    assert_eq!(kpi(&s, "KPI-01")["status"], "NO_DENOMINATOR");
    assert_eq!(kpi(&s, "KPI-07")["status"], "MANUAL_SAMPLE_REQUIRED");
    assert_eq!(
        kpi(&s, "KPI-02")["segments"]["riskTier"],
        "NOT_RECORDED_LOCALLY"
    );

    // Finance staff can list snapshots; the stored digest is returned unchanged.
    let list = w.wh.kpi_snapshots(&w.controls, &w.finance).await?;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["digest"], s["digest"]);

    let m = &w.db.migrator;
    // Snapshots are immutable and not readable or writable by the Data role directly.
    assert!(
        sqlx::query("UPDATE analytics.kpi_snapshots SET definitions_version='x'")
            .execute(m)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT count(*) FROM analytics.kpi_snapshots")
            .execute(&w.data_pool)
            .await
            .is_err()
    );
    close(w).await
}
