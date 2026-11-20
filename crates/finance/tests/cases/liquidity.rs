use super::*;
use serde_json::Value;

fn evidence(
    version: i64,
    available: &str,
    stale: bool,
) -> bank_finance::liquidity::LiquidityEvidence {
    let mut e = cash(version, stale);
    e.available = available.into();
    e.reserved = "0".into();
    e.settled = available.into();
    e
}
fn err(r: Result<Value>) -> String {
    match r {
        Ok(v) => panic!("expected an error, got {v}"),
        Err(e) => format!("{e:#}"),
    }
}
/// 21 business days of settlement outflow before today: 10,000 / 11,000 / 12,000 minor units in rotation.
async fn seed_history(f: &Fixture) -> Result<()> {
    let m = &f.db.migrator;
    let run = Uuid::new_v4();
    sqlx::query("INSERT INTO analytics.ingestion_runs VALUES($1,'ledger_entries',1,'BACKFILL',now()-interval '30 days',now(),21,21,0,0,0,now(),NULL,NULL,now(),now())")
        .bind(run).execute(m).await?;
    for o in 1..=21i64 {
        let day = Utc::now().date_naive() - Duration::days(o);
        let amount = 10_000 + (o % 3) * 1_000;
        let account = Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?;
        let operation = Uuid::new_v4();
        let record = json!({"batchId":Uuid::new_v4(),"operationId":operation,"postingKind":"EXTERNAL_PAYMENT","accountId":account,
            "accountKind":"SETTLEMENT","amountMinor":amount.to_string(),"currency":"TST","businessDate":day});
        sqlx::query("INSERT INTO analytics.facts(dataset,schema_version,economic_key,economic_digest,business_date,account_id,record,source_ref,source_recorded_at,run_id) VALUES('ledger_entries',1,$1,'d',$2,$3,$4,$5,now(),$6)")
            .bind(format!("{operation}|EXTERNAL_PAYMENT|{account}")).bind(day).bind(account).bind(record).bind(format!("fixture:{o}")).bind(run)
            .execute(m).await?;
    }
    sqlx::query("INSERT INTO analytics.watermarks VALUES('ledger_entries',now(),$1,now(),now()) ON CONFLICT(dataset) DO UPDATE SET checked_at=now()")
        .bind(run).execute(m).await?;
    Ok(())
}
async fn gate(f: &Fixture) -> Result<bool> {
    // Read exactly as Payments does, under the Payments role.
    let payments = f.db.module(Module::Payments).await?;
    let restricted =
        sqlx::query_scalar("SELECT finance.liquidity_restricted('EXTERNAL_SUBMISSION')")
            .fetch_one(&payments)
            .await?;
    payments.close().await;
    Ok(restricted)
}
async fn dual(f: &Fixture) -> Result<ActorContext> {
    let a = actor("dual-role");
    let person = PartyId::new();
    f.f.controls.link_person(&a, person).await?;
    for role in ["finance_adjuster", "finance_approver"] {
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')")
            .bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&f.db.migrator).await?;
    }
    Ok(a)
}

#[tokio::test]
async fn liquidity_forecasts_restrict_on_breach_escalate_reproduce_and_automate_only_with_independent_confirmed_approval()
-> Result<()> {
    let f = Fixture::new().await?;
    let _server = f.server()?;
    f.wait().await?;
    let dual = dual(&f).await?;

    // 1. No signed evidence: degraded, never confirmed funds, escalated once per episode, nothing restricted or automated.
    let first = f.f.run_forecast(&f.maker).await?;
    assert_eq!(first["status"], "UNCONFIRMED");
    assert_eq!(first["outcome"], "DEGRADED");
    assert!(first["output"]["confirmedAvailableMinor"].is_null());
    assert!(first["output"]["projectedLowMinor"].is_null());
    let flags = first["output"]["flags"].to_string();
    assert!(
        flags.contains("LIQUIDITY_MISSING")
            && flags.contains("WAREHOUSE_NEVER_INGESTED")
            && flags.contains("INSUFFICIENT_HISTORY")
    );
    assert_eq!(first["escalation"]["kind"], "DEGRADED");
    assert_eq!(first["escalation"]["reused"], false);
    assert!(first["restrictionId"].is_null() && first["action"].is_null());
    let second = f.f.run_forecast(&f.checker).await?;
    assert_eq!(second["escalation"]["reused"], true);
    assert_eq!(
        second["escalation"]["caseId"],
        first["escalation"]["caseId"]
    );
    let case: (String, String, String) =
        sqlx::query_as("SELECT kind,queue,severity FROM controls.cases WHERE id=$1")
            .bind(Uuid::parse_str(
                first["escalation"]["caseId"].as_str().unwrap(),
            )?)
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(
        case,
        (
            "FINANCE".into(),
            "finance-operations".into(),
            "CRITICAL".into()
        )
    );
    assert!(!gate(&f).await?);

    // 2. Fresh evidence and full history: a confirmed breach restricts, escalates and proposes one capped action.
    seed_history(&f).await?;
    publish_cash(&f, &evidence(1, "105000", false)).await?;
    let breach = f.f.run_forecast(&f.maker).await?;
    assert_eq!(breach["status"], "CONFIRMED", "{breach}");
    assert_eq!(breach["outcome"], "BREACH");
    let out = &breach["output"];
    assert_eq!(out["confirmedAvailableMinor"], "105000");
    assert_eq!(out["expectedOutflowMinor"], "11072");
    assert_eq!(out["daysWithData"], 14);
    let low: i64 = out["projectedLowMinor"].as_str().unwrap().parse()?;
    let high: i64 = out["highOutflowMinor"].as_str().unwrap().parse()?;
    assert_eq!(low, 105_000 - high);
    assert!(low < 100_000 && high > 11_072);
    assert_eq!(out["prefundingMinor"], (250_000 - low).to_string());
    assert_eq!(out["backtest"]["days"], 7);
    assert_eq!(out["backtest"]["observedDays"], 7);
    assert_eq!(out["backtest"]["baseline"], "PREVIOUS_DAY_ACTUAL");
    assert_eq!(breach["escalation"]["kind"], "BREACH");
    assert!(breach["restrictionId"].is_string());
    assert_eq!(breach["action"]["state"], "PROPOSED");
    assert!(
        gate(&f).await?,
        "breach must restrict new external submissions"
    );
    let breach_id = Uuid::parse_str(breach["id"].as_str().unwrap())?;
    let action = Uuid::parse_str(breach["action"]["id"].as_str().unwrap())?;
    let restriction = Uuid::parse_str(breach["restrictionId"].as_str().unwrap())?;

    // 3. Reproduction recomputes from stored inputs at the stored fact cut, also after later facts arrive.
    assert_eq!(
        f.f.reproduce_forecast(&f.checker, breach_id).await?["matched"],
        true
    );
    sqlx::query("INSERT INTO analytics.facts(dataset,schema_version,economic_key,economic_digest,business_date,account_id,record,source_ref,source_recorded_at,run_id) SELECT dataset,schema_version,economic_key||':late','d',business_date,account_id,record,source_ref||':late',now(),run_id FROM analytics.facts WHERE source_ref='fixture:1'")
        .execute(&f.db.migrator).await?;
    assert_eq!(
        f.f.reproduce_forecast(&f.maker, breach_id).await?["matched"],
        true
    );

    // 4. Maker/checker: roles and independence are enforced; nothing is booked before approval.
    let batches_before: i64 = sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
        .fetch_one(&f.db.migrator)
        .await?;
    assert!(err(f.f.confirm_liquidity_action(&f.checker, action).await).contains("role"));
    assert!(
        err(f.f.decide_liquidity_action(&f.checker, action, true).await)
            .contains("Maker confirmation required")
    );
    assert!(
        err(f.f.execute_liquidity_action(&f.maker, action).await)
            .contains("Approved liquidity action required")
    );
    assert_eq!(
        f.f.confirm_liquidity_action(&dual, action).await?["state"],
        "CONFIRMED"
    );
    assert!(
        err(f.f.decide_liquidity_action(&dual, action, true).await)
            .contains("Independent approver required")
    );
    assert!(err(f.f.decide_liquidity_action(&f.maker, action, true).await).contains("role"));
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core.journal_batches")
        .fetch_one(&f.db.migrator)
        .await?;
    assert_eq!(after, batches_before);

    // 5. Independent approval books exactly once through Core; replay returns the same batch; tampering is refused.
    let booked =
        f.f.decide_liquidity_action(&f.checker, action, true)
            .await?;
    assert_eq!(booked["state"], "EXECUTED", "{booked}");
    assert_eq!(booked["created"], true);
    let batch = Uuid::parse_str(booked["batchId"].as_str().unwrap())?;
    let legs: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT account_id,delta FROM core.journal_entries WHERE batch_id=$1 ORDER BY delta",
    )
    .bind(batch)
    .fetch_all(&f.db.migrator)
    .await?;
    let amount = 250_000 - low;
    assert_eq!(
        legs,
        vec![
            (
                Uuid::parse_str("4f1a780b-3e31-48ef-a294-00000000f1d0")?,
                -amount
            ),
            (
                Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?,
                amount
            )
        ]
    );
    let kind: String =
        sqlx::query_scalar("SELECT posting_kind FROM core.journal_batches WHERE id=$1")
            .bind(batch)
            .fetch_one(&f.db.migrator)
            .await?;
    assert_eq!(kind, "LIQUIDITY_PREFUNDING");
    let replay = f.f.execute_liquidity_action(&f.maker, action).await?;
    assert_eq!(replay["created"], false);
    assert_eq!(replay["batchId"], booked["batchId"]);
    let (details, digest, maker, checker): (Value, String, Uuid, Uuid) = sqlx::query_as("SELECT details,details_digest,maker_person,checker_person FROM core.liquidity_action_receipts WHERE execution_id=$1")
        .bind(action).fetch_one(&f.db.migrator).await?;
    let mut tampered = details.clone();
    tampered["executionId"] = json!(Uuid::new_v4());
    tampered["amountMinor"] = json!("9999999");
    let digest2: String = sqlx::query_scalar("SELECT finance.digest($1)")
        .bind(&tampered)
        .fetch_one(&f.db.migrator)
        .await?;
    let refused =
        f.f.corrections
            .execute_liquidity_action(&tampered, &digest2, maker, checker)
            .await;
    assert!(format!("{:#}", refused.unwrap_err()).contains("Approved liquidity action required"));
    let swapped =
        f.f.corrections
            .execute_liquidity_action(&details, &digest, checker, maker)
            .await;
    assert!(format!("{:#}", swapped.unwrap_err()).contains("identity conflict"));

    // 6. Stale evidence supersedes a newer proposal: missing or stale evidence never justifies an automated action.
    let again = f.f.run_forecast(&f.maker).await?;
    assert_eq!(again["outcome"], "BREACH");
    assert_eq!(
        again["escalation"]["reused"], true,
        "the active restriction's case is reused"
    );
    let action2 = Uuid::parse_str(again["action"]["id"].as_str().unwrap())?;
    publish_cash(&f, &evidence(2, "105000", true)).await?;
    let stale = f.f.run_forecast(&f.maker).await?;
    assert_eq!(stale["status"], "UNCONFIRMED");
    assert!(
        stale["output"]["flags"]
            .to_string()
            .contains("LIQUIDITY_STALE")
    );
    assert!(
        err(f.f.confirm_liquidity_action(&f.maker, action2).await)
            .contains("Current confirmed breach forecast required")
    );
    let view = f.f.liquidity_view(&f.checker).await?;
    let states: Vec<String> = view["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["state"].as_str().unwrap().to_owned())
        .collect();
    assert!(states.contains(&"SUPERSEDED".to_owned()) && states.contains(&"EXECUTED".to_owned()));
    assert_eq!(view["restricted"], true);

    // 7. Versioned policy: valid documents only, independent approval, the new version drives later forecasts.
    let mut doc: Value =
        sqlx::query_scalar("SELECT document FROM finance.liquidity_policies WHERE version=1")
            .fetch_one(&f.db.migrator)
            .await?;
    let mut bad = doc.clone();
    bad["restrictedActions"] = json!([]);
    assert!(
        err(f.f.propose_liquidity_policy(&f.maker, &bad).await)
            .contains("Invalid liquidity policy document")
    );
    doc["limits"]["maxActionMinor"] = json!("1000");
    let v2 = f.f.propose_liquidity_policy(&f.maker, &doc).await?;
    assert_eq!(v2["version"], 2);
    assert!(err(f.f.decide_liquidity_policy(&f.maker, 2, true).await).contains("role"));
    let v3 = f.f.propose_liquidity_policy(&dual, &doc).await?;
    assert!(
        err(f
            .f
            .decide_liquidity_policy(&dual, v3["version"].as_i64().unwrap() as i32, true)
            .await)
        .contains("Independent approver required")
    );
    assert_eq!(
        f.f.decide_liquidity_policy(&f.checker, 2, true).await?["activeVersion"],
        2
    );

    // 8. Lift: maker proposes; the approver can apply only on a current confirmed within-limits forecast.
    let lift = f.f.propose_restriction_lift(&f.maker, restriction).await?;
    let lift_id = Uuid::parse_str(lift["liftId"].as_str().unwrap())?;
    assert!(
        err(f.f.propose_restriction_lift(&f.maker, restriction).await).contains("already pending")
    );
    assert!(
        err(f.f.decide_restriction_lift(&f.checker, lift_id, true).await)
            .contains("Current confirmed within-limits forecast required")
    );
    publish_cash(&f, &evidence(3, "105000", false)).await?;
    let capped = f.f.run_forecast(&f.maker).await?;
    assert_eq!(capped["policyVersion"], 2);
    assert_eq!(capped["output"]["prefundingMinor"], "1000");
    assert!(
        capped["output"]["flags"]
            .to_string()
            .contains("ACTION_CAPPED_BY_LIMIT")
    );
    assert!(
        err(f.f.decide_restriction_lift(&f.checker, lift_id, true).await).contains("within-limits")
    );
    publish_cash(&f, &evidence(4, "900000", false)).await?;
    let healthy = f.f.run_forecast(&f.checker).await?;
    assert_eq!(healthy["outcome"], "WITHIN_LIMITS");
    assert!(healthy["escalation"].is_null());
    assert!(
        gate(&f).await?,
        "a healthy forecast alone does not lift the restriction"
    );
    assert!(err(f.f.decide_restriction_lift(&f.maker, lift_id, true).await).contains("role"));
    let lifted =
        f.f.decide_restriction_lift(&f.checker, lift_id, true)
            .await?;
    assert_eq!(lifted["outcome"], "APPLIED");
    assert_eq!(lifted["restricted"], false);
    assert!(!gate(&f).await?);

    // 9. History is immutable and the Finance role cannot write liquidity records directly.
    assert!(
        sqlx::query("UPDATE finance.liquidity_forecasts SET outcome='WITHIN_LIMITS'")
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM finance.liquidity_restrictions")
            .execute(&f.db.migrator)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO finance.liquidity_lift_decisions(lift_id,restriction_id,outcome,decided_by,decided_person) VALUES($1,$2,'APPLIED','{}',$1)")
        .bind(Uuid::new_v4()).bind(restriction).execute(&f.f.pool).await.is_err());
    assert!(
        sqlx::query("UPDATE finance.liquidity_policy_head SET active_version=1")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM controls.audit_events WHERE stream='liquidity'")
            .fetch_one(&f.db.migrator)
            .await?;
    assert!(audited >= 15, "{audited}");
    f.close().await
}
