//! WP-38 audiences, insights and the local journey-partner simulator role: consent at build, export and delivery;
//! withdrawal hides optional insights without deleting evidence; partners can only advise; outcome reports disclose
//! denominators, watermarks and late data.
use super::analytics::{actor, close, project, world};
use bank_adapters::journeys::{self, LOCAL_PARTNER};
use bank_data::{analytics::Denied, audience::Audiences, insights::Insights};
use bank_platform::domain::{Currency, Money, TrustContext};
use serde_json::{Value, json};
use uuid::Uuid;

async fn consent(w: &super::analytics::World, party: Uuid, enabled: bool) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO data.notification_preferences VALUES($1,'MARKETING',$2,'{\"fixture\":true}',now()) ON CONFLICT(person_id,purpose) DO UPDATE SET enabled=EXCLUDED.enabled,updated_at=now()")
        .bind(party)
        .bind(enabled)
        .execute(&w.db.migrator)
        .await?;
    Ok(())
}
fn denied(error: anyhow::Error) -> String {
    error
        .downcast_ref::<Denied>()
        .map(|d| d.0.clone())
        .unwrap_or_else(|| panic!("expected a recorded denial, got {error:#}"))
}
async fn recommend(a: &Audiences, document: Value) -> anyhow::Result<(Vec<u8>, Value)> {
    let body = serde_json::to_vec(&document)?;
    let signature = journeys::sign(LOCAL_PARTNER, &body).unwrap();
    let (raw, signed) = a
        .partner_recommendation(LOCAL_PARTNER, &body, &signature)
        .await?;
    assert!(journeys::verify(LOCAL_PARTNER, &raw, &signed));
    let parsed = serde_json::from_slice(&raw)?;
    Ok((raw, parsed))
}
async fn fetch(a: &Audiences, export: Uuid) -> anyhow::Result<(Vec<u8>, Value)> {
    let signature = journeys::sign(
        LOCAL_PARTNER,
        &journeys::export_request(LOCAL_PARTNER, &export.to_string()),
    )
    .unwrap();
    let (raw, _) = a.partner_export(LOCAL_PARTNER, export, &signature).await?;
    let parsed = serde_json::from_slice(&raw)?;
    Ok((raw, parsed))
}
fn titles(view: &Value) -> Vec<String> {
    view["insights"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn consented_audiences_minimized_exports_advisory_partners_and_withdrawal_preserve_evidence()
-> anyhow::Result<()> {
    let w = world().await?;
    let audiences = Audiences::new(w.data_pool.clone());
    let insights = Insights::new(w.data_pool.clone());
    let alice_customer = actor("alice", TrustContext::Customer);
    let bob_customer = actor("bob", TrustContext::Customer);
    w.wh.ingest_due(0.0).await?;
    consent(&w, w.alice, true).await?;
    // Only alice permits marketing; both were credited recently, so both are candidates.
    assert_eq!(
        denied(
            audiences
                .build(&w.controls, &w.finance, "savings_starter", 1)
                .await
                .unwrap_err()
        ),
        "ROLE_NOT_PERMITTED_FOR_PURPOSE"
    );
    let audience = audiences
        .build(&w.controls, &w.analyst, "savings_starter", 1)
        .await?;
    assert_eq!(
        (
            audience["candidates"].as_i64(),
            audience["members"].as_i64()
        ),
        (Some(2), Some(1)),
        "{audience:#}"
    );
    assert_eq!(audience["excluded"]["noMarketingConsent"], 1);
    assert_eq!(audience["segmentDigest"].as_str().unwrap().len(), 64);
    let audience_id: Uuid = serde_json::from_value(audience["id"].clone())?;
    let alice_view = insights.list(&w.controls, &alice_customer).await?;
    assert_eq!(titles(&alice_view), vec!["Put part of each deposit aside"]);
    assert!(
        alice_view["insights"][0]["why"]
            .as_str()
            .unwrap()
            .contains("savings_starter_tip v1")
    );
    assert_eq!(alice_view["insights"][0]["actionRoute"], "#/accounts");
    assert!(titles(&insights.list(&w.controls, &bob_customer).await?).is_empty());
    // Service outreach is never exported to a partner.
    let service = audiences
        .build(&w.controls, &w.analyst, "dormant_check_in", 1)
        .await?;
    let service_id: Uuid = serde_json::from_value(service["id"].clone())?;
    assert_eq!(
        denied(
            audiences
                .export(&w.controls, &w.analyst, service_id, LOCAL_PARTNER)
                .await
                .unwrap_err()
        ),
        "PURPOSE_NOT_PERMITTED_FOR_PARTNER"
    );
    // Export carries pseudonymous tokens only; fetching needs the partner signature.
    let export = audiences
        .export(&w.controls, &w.analyst, audience_id, LOCAL_PARTNER)
        .await?;
    assert_eq!(export["exported"], 1);
    let export_id: Uuid = serde_json::from_value(export["exportId"].clone())?;
    assert!(
        audiences
            .partner_export(LOCAL_PARTNER, export_id, "00")
            .await
            .is_err()
    );
    assert!(
        audiences
            .partner_export("OTHER_PARTNER", export_id, "00")
            .await
            .is_err()
    );
    let (raw, delivered) = fetch(&audiences, export_id).await?;
    let text = String::from_utf8(raw)?;
    for secret in [
        w.alice.to_string(),
        w.bob.to_string(),
        w.accounts[0].0.to_string(),
    ] {
        assert!(!text.contains(&secret), "export must not reveal {secret}");
    }
    let token = delivered["members"][0].as_str().unwrap().to_owned();
    assert!(token.starts_with("m_"));
    // An accepted recommendation becomes an advisory insight with partner provenance.
    let tip = json!({"messageId":"MSG-0000000001","memberToken":token,"recommendationCode":"SAVINGS_TIP","title":"A small weekly habit","body":"Many people move a fixed amount each week. You decide if that suits you.","actionRoute":"#/accounts"});
    let (first, accepted) = recommend(&audiences, tip.clone()).await?;
    assert_eq!(accepted["result"], "ACCEPTED");
    let (again, _) = recommend(&audiences, tip.clone()).await?;
    assert_eq!(
        first, again,
        "a duplicate message replays the stored response"
    );
    let mut reused = tip.clone();
    reused["title"] = json!("Different content");
    assert!(
        recommend(&audiences, reused).await.is_err(),
        "message id reuse with other content"
    );
    assert_eq!(
        titles(&insights.list(&w.controls, &alice_customer).await?).len(),
        2
    );
    // Advisory boundary: anything beyond advice is rejected and recorded, and nothing outside analytics changes.
    let state = |m: &sqlx::PgPool| {
        let m = m.clone();
        async move {
            sqlx::query_as::<_, (i64, i64, i64, i64, i64)>("SELECT (SELECT count(*) FROM core.journal_entries),(SELECT coalesce(sum(booked),0)::bigint FROM core.ledger_accounts),(SELECT count(*) FROM controls.role_grants),(SELECT count(*) FROM journeys.applications),(SELECT count(*) FROM core.holds)")
                .fetch_one(&m)
                .await
        }
    };
    let before = state(&w.db.migrator).await?;
    for (n, (document, reason)) in [
        (json!({"memberToken":token,"recommendationCode":"SAVINGS_TIP","title":"Approved","body":"Pre-approved.","approve":true}), "FIELDS_NOT_PERMITTED"),
        (json!({"memberToken":token,"recommendationCode":"SAVINGS_TIP","title":"Limit","body":"Raise limit.","creditLimitMinor":"500000"}), "FIELDS_NOT_PERMITTED"),
        (json!({"memberToken":token,"recommendationCode":"APPROVE_APPLICATION","title":"Approve","body":"Approve this customer."}), "RECOMMENDATION_NOT_PERMITTED"),
        (json!({"memberToken":token,"recommendationCode":"BUDGET_REMINDER","title":"Move money","body":"Transfer now.","actionRoute":"#/staff/roles"}), "ACTION_NOT_PERMITTED"),
        (json!({"memberToken":token,"recommendationCode":"BUDGET_REMINDER","title":"Hi","body":"<script>alert(1)</script> tip"}), "CONTENT_NOT_PERMITTED"),
        (json!({"memberToken":"m_00000000000000000000000000000000","recommendationCode":"BUDGET_REMINDER","title":"Budget","body":"Review your budget."}), "UNKNOWN_MEMBER"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut document = document;
        document["messageId"] = json!(format!("MSG-REJECT-{n:03}"));
        let (_, result) = recommend(&audiences, document).await?;
        assert_eq!((result["result"].as_str(), result["reason"].as_str()), (Some("REJECTED"), Some(reason)));
    }
    assert_eq!(
        state(&w.db.migrator).await?,
        before,
        "partners cannot approve, entitle or post"
    );
    assert_eq!(
        titles(&insights.list(&w.controls, &alice_customer).await?).len(),
        2
    );
    // Withdrawal: optional insights hidden, delivery withheld, new advice suppressed, new exports exclude; evidence kept.
    consent(&w, w.alice, false).await?;
    let hidden = insights.list(&w.controls, &alice_customer).await?;
    assert_eq!(hidden["marketingPermitted"], false);
    assert!(titles(&hidden).is_empty());
    let (_, withheld) = fetch(&audiences, export_id).await?;
    assert_eq!(
        (
            withheld["members"].as_array().unwrap().len(),
            withheld["withheld"].as_i64()
        ),
        (0, Some(1))
    );
    let (_, suppressed) = recommend(&audiences, json!({"messageId":"MSG-0000000002","memberToken":token,"recommendationCode":"BUDGET_REMINDER","title":"Budget","body":"Review your budget."})).await?;
    assert_eq!(suppressed["result"], "SUPPRESSED");
    let second = audiences
        .export(&w.controls, &w.analyst, audience_id, LOCAL_PARTNER)
        .await?;
    assert_eq!(
        (
            second["exported"].as_i64(),
            second["withdrawnExcluded"].as_i64()
        ),
        (Some(0), Some(1))
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM analytics.insights WHERE party_id=$1")
            .bind(w.alice)
            .fetch_one(&w.db.migrator)
            .await?,
        2,
        "withdrawal never deletes evidence"
    );
    assert!(
        sqlx::query("DELETE FROM analytics.insights")
            .execute(&w.db.migrator)
            .await
            .is_err()
    );
    // Customers dismiss only their own insights.
    consent(&w, w.alice, true).await?;
    let shown = insights.list(&w.controls, &alice_customer).await?;
    let first_id: Uuid = serde_json::from_value(shown["insights"][0]["id"].clone())?;
    assert!(
        insights
            .dismiss(&w.controls, &bob_customer, first_id)
            .await
            .is_err()
    );
    assert_eq!(
        titles(
            &insights
                .dismiss(&w.controls, &alice_customer, first_id)
                .await?
        )
        .len(),
        1
    );
    // Outcome report: explicit denominators; late (projected but not ingested) data is disclosed.
    w.core
        .transfer(
            Uuid::new_v4(),
            w.accounts[1],
            w.accounts[0],
            Money::new(10, Currency::synthetic()),
            None,
        )
        .await?;
    project(&w).await?;
    let late = audiences
        .report(&w.controls, &w.analyst, audience_id)
        .await?;
    assert_eq!(late["status"], "INCOMPLETE");
    assert_eq!(late["completeness"][0]["code"], "LATE_DATA");
    assert_eq!(late["outcomes"]["creditedAfterBuild"], 0);
    w.wh.ingest_due(0.0).await?;
    let report = audiences
        .report(&w.controls, &w.analyst, audience_id)
        .await?;
    assert_eq!(report["status"], "COMPLETE", "{report:#}");
    assert_eq!(report["denominators"]["candidates"], 2);
    assert_eq!(report["denominators"]["members"], 1);
    assert_eq!(report["denominators"]["exported"], 1);
    assert_eq!(report["denominators"]["withdrawnSinceBuild"], 0);
    assert_eq!(report["outcomes"]["creditedAfterBuild"], 1);
    assert_eq!(report["outcomes"]["partnerRecommendationsAccepted"], 1);
    assert_eq!(report["outcomes"]["insightsDismissed"], 1);
    assert!(report["watermarks"]["ledger_entries"]["lastRunAt"].is_string());
    let view = audiences.view(&w.controls, &w.steward).await?;
    assert_eq!(view["partners"][0]["status"], "LOCAL_TEMPLATE");
    assert!(
        view["partnerMessages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["reason"] == "FIELDS_NOT_PERMITTED")
    );
    assert_eq!(
        view["audiences"][0]["reports"].as_array().unwrap().len()
            + view["audiences"][1]["reports"].as_array().unwrap().len(),
        2
    );
    assert!(audiences.view(&w.controls, &w.finance).await.is_err());
    close(w).await
}
