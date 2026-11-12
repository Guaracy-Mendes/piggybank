use super::*;
use bank_finance::{
    group_reconciliation::GroupInput,
    settlement_set::{PublishSet, SetKind, SettlementSet},
};
use sqlx::Row;
impl PaymentFixture {
    async fn publish_set(
        &self,
        input: &PublishSet,
    ) -> Result<(reqwest::StatusCode, Option<SettlementSet>)> {
        let response = reqwest::Client::new()
            .post(format!("{}/admin/settlement-sets", self.f.base))
            .header("X-Finance-Key", bank_finance::protocol::signing_key())
            .json(input)
            .send()
            .await?;
        let status = response.status();
        Ok((
            status,
            if status.is_success() {
                Some(response.json().await?)
            } else {
                None
            },
        ))
    }
    fn set(&self, kind: SetKind, files: Vec<Uuid>, cutoff: chrono::DateTime<Utc>) -> PublishSet {
        PublishSet {
            set_id: Uuid::new_v4(),
            kind,
            range: range(),
            generated_at: cutoff,
            file_ids: files,
        }
    }
    async fn group(
        &self,
        report: Uuid,
        set: Uuid,
    ) -> Result<bank_finance::group_reconciliation::GroupDetail> {
        self.f
            .f
            .create_group_reconciliation(
                &self.f.maker,
                Uuid::new_v4(),
                GroupInput {
                    report_id: report,
                    set_id: set,
                },
            )
            .await
    }
}
#[tokio::test]
async fn group_reconciliation_matches_more_than_one_hundred_outgoing_rows_and_keeps_v1_artifacts()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let mut rows = vec![];
    for n in 0..105 {
        let r = x
            .instruction(true, if n == 0 { 9007199254740993 } else { 17 })
            .await?;
        rows.push(x.settle(&r, true).await?.unwrap());
    }
    let report = x.report(true).await?;
    let cutoff = Utc::now();
    let mut files = vec![];
    for chunk in rows.chunks(100) {
        let mut f = x.file(chunk.to_vec());
        f.generated_at = cutoff;
        assert_eq!(x.publish_file(&f).await?, reqwest::StatusCode::CREATED);
        files.push(f);
    }
    let old =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: files[0].file_id,
                },
            )
            .await?;
    assert_eq!(old.reconciliation.exception_count, 5);
    let input = x.set(
        SetKind::Outgoing,
        files.iter().map(|f| f.file_id).collect(),
        cutoff,
    );
    let (status, manifest) = x.publish_set(&input).await?;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    let manifest = manifest.unwrap();
    assert_eq!(manifest.row_count, 105);
    assert_eq!(manifest.members.len(), 2);
    let state = x.money_state().await?;
    let key = Uuid::new_v4();
    let capture = GroupInput {
        report_id: report,
        set_id: input.set_id,
    };
    let (a, b) = tokio::try_join!(
        x.f.f
            .create_group_reconciliation(&x.f.maker, key, capture.clone()),
        x.f.f
            .create_group_reconciliation(&x.f.maker, key, capture.clone())
    )?;
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.reconciliation.completeness, "COMPLETE");
    assert_eq!(a.reconciliation.matched_count, 105);
    assert_eq!(a.reconciliation.core_principal_total, "9007199254742761");
    assert_eq!(a.reconciliation.core_bank_fee_total.as_deref(), Some("735"));
    assert_eq!(
        a.reconciliation.provider_totals[0].principal,
        "9007199254742761"
    );
    assert!(a.issues.is_empty());
    let first =
        x.f.f
            .group_reconciliation_items(&x.f.checker, a.reconciliation.id, 0, 100)
            .await?;
    assert_eq!(first.items.len(), 100);
    assert_eq!(first.next_after, Some(100));
    let last =
        x.f.f
            .group_reconciliation_items(&x.f.maker, a.reconciliation.id, 100, 100)
            .await?;
    assert_eq!(last.items.len(), 5);
    assert!(last.next_after.is_none());
    assert_eq!(first.reconciliation_digest, last.reconciliation_digest);
    for item in first.items.iter().chain(&last.items) {
        assert_eq!(item.source_rows.len(), 1);
        assert!(item.outgoing.is_some());
        assert!(item.credit.is_none());
        let source = &item.source_rows[0];
        let f = files.iter().find(|f| f.file_id == source.file_id).unwrap();
        assert_eq!(
            fingerprint(&f.rows[source.row_ordinal as usize - 1])?,
            source.row_digest
        );
    }
    assert!(!serde_json::to_string(&first)?.contains("signature"));
    assert_eq!(state, x.money_state().await?);
    assert_eq!(
        x.f.f
            .reconciliation_detail(&x.f.maker, old.reconciliation.id)
            .await?
            .digest,
        old.digest
    );
    assert!(
        x.f.f
            .create_group_reconciliation(&x.f.checker, Uuid::new_v4(), capture.clone())
            .await
            .is_err()
    );
    assert!(
        x.f.f
            .create_group_reconciliation(
                &x.f.maker,
                key,
                GroupInput {
                    report_id: report,
                    set_id: Uuid::new_v4()
                }
            )
            .await
            .err()
            .unwrap()
            .is::<bank_finance::reconciliation::ReconciliationKeyConflict>()
    );
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE role='finance_adjuster'")
        .execute(&x.f.db.migrator)
        .await?;
    assert!(
        x.f.f
            .create_group_reconciliation(&x.f.maker, key, capture)
            .await
            .is_err()
    );
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn group_reconciliation_matches_incoming_and_partial_returns_across_files_and_owns_suspense()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let original = x.instruction(true, 1000).await?;
    x.settle(&original, true).await?;
    let incoming = x.credit(9007199254740993, None, false).await?;
    let partial = x.credit(400, Some(original.operation_id), false).await?;
    let rest = x.credit(600, Some(original.operation_id), false).await?;
    let unmatched = x.credit(101, None, true).await?;
    for e in [&incoming, &partial, &rest] {
        x.receive_credit(e, true, true).await?;
    }
    x.receive_credit(&unmatched, false, true).await?;
    let cutoff = Utc::now();
    let mut f1 = x.credit_file(&[incoming, partial]);
    let mut f2 = x.credit_file(&[rest, unmatched]);
    f1.generated_at = cutoff;
    f2.generated_at = cutoff;
    for f in [&f1, &f2] {
        assert_eq!(
            x.publish_credit_file(f).await?,
            reqwest::StatusCode::CREATED
        );
    }
    let report = x.report(true).await?;
    let input = x.set(SetKind::Credits, vec![f2.file_id, f1.file_id], cutoff);
    assert_eq!(x.publish_set(&input).await?.0, reqwest::StatusCode::CREATED);
    let state = x.money_state().await?;
    let run = x.group(report, input.set_id).await?;
    assert_eq!(run.reconciliation.matched_count, 3);
    assert_eq!(run.reconciliation.suspense_count, 1);
    assert_eq!(run.reconciliation.completeness, "PARTIAL");
    assert_eq!(run.reconciliation.core_principal_total, "9007199254742094");
    assert_eq!(
        run.reconciliation.core_allocated_principal_total.as_deref(),
        Some("9007199254741993")
    );
    assert_eq!(
        run.reconciliation.core_suspense_principal_total.as_deref(),
        Some("101")
    );
    assert!(run.reconciliation.core_bank_fee_total.is_none());
    assert!(
        run.issues
            .iter()
            .any(|i| i.code == "FUNDS_IN_SUSPENSE" && i.case_id.is_some())
    );
    let page =
        x.f.f
            .group_reconciliation_items(&x.f.maker, run.reconciliation.id, 0, 100)
            .await?;
    for item in page
        .items
        .iter()
        .filter_map(|i| i.credit.as_ref())
        .filter(|i| i.original_payment_id.is_some())
    {
        assert_eq!(item.returned_principal_total.as_deref(), Some("1000"));
        assert!(item.original_batch_id.is_some());
    }
    assert_eq!(state, x.money_state().await?);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn group_reconciliation_retains_cross_file_duplicate_and_currency_fee_claims_with_provenance()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let e = x.credit(123, None, false).await?;
    x.receive_credit(&e, true, true).await?;
    let report = x.report(true).await?;
    let cutoff = Utc::now();
    let mut first = x.credit_file(&[e]);
    first.generated_at = cutoff;
    let mut second = first.clone();
    second.file_id = Uuid::new_v4();
    second.rows[0].provider_fee_minor = "5".into();
    second.rows[0].currency_code = "USD".into();
    for f in [&first, &second] {
        assert_eq!(
            x.publish_credit_file(f).await?,
            reqwest::StatusCode::CREATED
        );
    }
    let input = x.set(
        SetKind::Credits,
        vec![first.file_id, second.file_id],
        cutoff,
    );
    x.publish_set(&input).await?;
    let state = x.money_state().await?;
    let run = x.group(report, input.set_id).await?;
    assert_eq!(run.reconciliation.exception_count, 1);
    for code in [
        "DUPLICATE_PROVIDER_LINE",
        "DUPLICATE_PROVIDER_EVIDENCE",
        "DUPLICATE_PROVIDER_MOVEMENT",
        "PROVIDER_CURRENCY_MISMATCH",
        "PROVIDER_FEE_UNEXPECTED",
    ] {
        assert!(
            run.issues
                .iter()
                .any(|i| i.code == code && i.case_id.is_some()),
            "{code}"
        );
    }
    assert_eq!(run.reconciliation.provider_totals.len(), 2);
    assert_eq!(run.reconciliation.provider_row_count, 2);
    let item =
        x.f.f
            .group_reconciliation_items(&x.f.maker, run.reconciliation.id, 0, 1)
            .await?
            .items
            .remove(0);
    assert_eq!(item.source_rows.len(), 2);
    assert_ne!(item.source_rows[0].file_id, item.source_rows[1].file_id);
    assert_eq!(item.credit.as_ref().unwrap().provider_rows.len(), 2);
    assert_eq!(state, x.money_state().await?);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn group_reconciliation_rejects_non_closed_manifests_wrong_period_cutoff_and_conflicting_replays()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let mut first = x.file(vec![]);
    let cutoff = first.generated_at;
    let mut second = x.file(vec![]);
    second.generated_at = cutoff;
    let mut credit = x.credit_file(&[]);
    credit.generated_at = cutoff;
    x.publish_file(&first).await?;
    x.publish_file(&second).await?;
    x.publish_credit_file(&credit).await?;
    let input = x.set(
        SetKind::Outgoing,
        vec![second.file_id, first.file_id],
        cutoff,
    );
    let (status, manifest) = x.publish_set(&input).await?;
    assert_eq!(status, reqwest::StatusCode::CREATED);
    assert!(
        manifest
            .unwrap()
            .members
            .windows(2)
            .all(|m| m[0].file_id < m[1].file_id)
    );
    let mut reversed = input.clone();
    reversed.file_ids.reverse();
    assert_eq!(x.publish_set(&reversed).await?.0, reqwest::StatusCode::OK);
    let mut conflict = input.clone();
    conflict.file_ids = vec![first.file_id];
    assert_eq!(
        x.publish_set(&conflict).await?.0,
        reqwest::StatusCode::CONFLICT
    );
    for ids in [
        vec![],
        vec![first.file_id, first.file_id],
        vec![Uuid::nil()],
        vec![Uuid::new_v4()],
        vec![credit.file_id],
        (0..11).map(|_| Uuid::new_v4()).collect(),
    ] {
        let bad = x.set(SetKind::Outgoing, ids, cutoff);
        assert_eq!(
            x.publish_set(&bad).await?.0,
            reqwest::StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let mut bad = x.set(
        SetKind::Outgoing,
        vec![first.file_id],
        cutoff - Duration::microseconds(1),
    );
    assert_eq!(
        x.publish_set(&bad).await?.0,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    bad.generated_at = cutoff;
    bad.range.from_date -= Duration::days(1);
    assert_eq!(
        x.publish_set(&bad).await?.0,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let report = x.report(true).await?;
    let run = x.group(report, input.set_id).await?;
    assert_eq!(run.reconciliation.completeness, "COMPLETE");
    assert_eq!(run.reconciliation.record_count, 0);
    let missing = x.group(report, Uuid::new_v4()).await;
    assert!(
        missing
            .err()
            .unwrap()
            .is::<bank_finance::settlement::SettlementUnavailable>()
    );
    first.file_id = Uuid::new_v4();
    first.generated_at -= Duration::seconds(1);
    x.publish_file(&first).await?;
    let mixed = x.set(
        SetKind::Outgoing,
        vec![first.file_id, second.file_id],
        cutoff,
    );
    assert_eq!(
        x.publish_set(&mixed).await?.0,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn group_reconciliation_missing_member_saves_nothing_and_archived_membership_damage_fails_closed()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let first = x.file(vec![]);
    let cutoff = first.generated_at;
    let mut second = x.file(vec![]);
    second.generated_at = cutoff;
    for f in [&first, &second] {
        x.publish_file(f).await?;
    }
    let set = x.set(
        SetKind::Outgoing,
        vec![first.file_id, second.file_id],
        cutoff,
    );
    x.publish_set(&set).await?;
    let report = x.report(true).await?;
    let run = x.group(report, set.set_id).await?;
    let state = x.money_state().await?;
    assert!(
        sqlx::query("UPDATE finance.group_reconciliation_runs SET digest='damaged' WHERE id=$1")
            .bind(run.reconciliation.id)
            .execute(&x.f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM finance.group_reconciliation_items WHERE run_id=$1")
            .bind(run.reconciliation.id)
            .execute(&x.f.f.pool)
            .await
            .is_err()
    );
    sqlx::query(
        "ALTER TABLE simulator.settlement_files DISABLE TRIGGER immutable_settlement_files",
    )
    .execute(&x.f.db.migrator)
    .await?;
    sqlx::query("DELETE FROM simulator.settlement_files WHERE id=$1")
        .bind(first.file_id)
        .execute(&x.f.db.migrator)
        .await?;
    sqlx::query("ALTER TABLE simulator.settlement_files ENABLE TRIGGER immutable_settlement_files")
        .execute(&x.f.db.migrator)
        .await?;
    assert!(
        x.group(report, set.set_id)
            .await
            .err()
            .unwrap()
            .is::<bank_finance::settlement::SettlementUnavailable>()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM finance.group_reconciliation_runs")
            .fetch_one(&x.f.db.migrator)
            .await?,
        1
    );
    assert_eq!(
        x.f.f
            .group_reconciliation_detail(&x.f.maker, run.reconciliation.id)
            .await?
            .digest,
        run.digest
    );
    let original: Value = sqlx::query_scalar(
        "SELECT file_packets FROM finance.group_reconciliation_runs WHERE id=$1",
    )
    .bind(run.reconciliation.id)
    .fetch_one(&x.f.db.migrator)
    .await?;
    for damaged in [
        json!([original[1].clone(), original[0].clone()]),
        json!([original[0].clone()]),
        json!([
            original[0].clone(),
            original[1].clone(),
            original[0].clone()
        ]),
    ] {
        sqlx::query("ALTER TABLE finance.group_reconciliation_runs DISABLE TRIGGER immutable_group_reconciliation_runs").execute(&x.f.db.migrator).await?;
        sqlx::query("UPDATE finance.group_reconciliation_runs SET file_packets=$2 WHERE id=$1")
            .bind(run.reconciliation.id)
            .bind(damaged)
            .execute(&x.f.db.migrator)
            .await?;
        sqlx::query("ALTER TABLE finance.group_reconciliation_runs ENABLE TRIGGER immutable_group_reconciliation_runs").execute(&x.f.db.migrator).await?;
        assert!(
            x.f.f
                .group_reconciliation_detail(&x.f.maker, run.reconciliation.id)
                .await
                .is_err()
        );
    }
    sqlx::query("ALTER TABLE finance.group_reconciliation_runs DISABLE TRIGGER immutable_group_reconciliation_runs").execute(&x.f.db.migrator).await?;
    sqlx::query("UPDATE finance.group_reconciliation_runs SET file_packets=$2 WHERE id=$1")
        .bind(run.reconciliation.id)
        .bind(original)
        .execute(&x.f.db.migrator)
        .await?;
    sqlx::query("ALTER TABLE finance.group_reconciliation_runs ENABLE TRIGGER immutable_group_reconciliation_runs").execute(&x.f.db.migrator).await?;
    assert_eq!(
        x.f.f
            .group_reconciliation_detail(&x.f.maker, run.reconciliation.id)
            .await?
            .digest,
        run.digest
    );
    assert_eq!(state, x.money_state().await?);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn group_reconciliation_preserves_suspense_and_late_report_membership_through_replay_and_recovery()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let e = x.credit(419, None, false).await?;
    x.receive_credit(&e, false, true).await?;
    let report = x.report(true).await?;
    let mut file = x.credit_file(std::slice::from_ref(&e));
    let cutoff = file.generated_at;
    let mut empty = x.credit_file(&[]);
    empty.generated_at = cutoff;
    for f in [&file, &empty] {
        x.publish_credit_file(f).await?;
    }
    let set = x.set(SetKind::Credits, vec![file.file_id, empty.file_id], cutoff);
    x.publish_set(&set).await?;
    let key = Uuid::new_v4();
    let input = GroupInput {
        report_id: report,
        set_id: set.set_id,
    };
    let original =
        x.f.f
            .create_group_reconciliation(&x.f.maker, key, input.clone())
            .await?;
    assert_eq!(original.reconciliation.suspense_count, 1);
    x.f.core.allocate_rail_credit(&e, &TrustedCredit).await?;
    sqlx::query(
        "UPDATE payments.credit_intents SET state='CREDITED',updated_at=now() WHERE movement_id=$1",
    )
    .bind(e.movement_id)
    .execute(&x.f.db.migrator)
    .await?;
    let late = x.group(report, set.set_id).await?;
    assert_eq!(late.reconciliation.timing_count, 1);
    assert_eq!(late.reconciliation.exception_count, 0);
    assert!(late.issues.iter().any(|i| i.code == "CORE_AFTER_REPORT"));
    let mut offline = x.f.f.clone();
    offline.client = GlClient::new("http://127.0.0.1:1")?;
    assert_eq!(
        offline
            .create_group_reconciliation(&x.f.maker, key, input)
            .await?
            .digest,
        original.digest
    );
    let fresh = x.report(true).await?;
    let matched = x.group(fresh, set.set_id).await?;
    assert_eq!(matched.reconciliation.matched_count, 1);
    assert_eq!(matched.reconciliation.completeness, "COMPLETE");
    assert_eq!(
        x.f.f
            .group_reconciliation_detail(&x.f.checker, original.reconciliation.id)
            .await?
            .digest,
        original.digest
    );
    assert!(
        x.f.f
            .group_reconciliation_items(&x.f.maker, matched.reconciliation.id, -1, 5)
            .await
            .is_err()
    );
    assert!(
        x.f.f
            .group_reconciliation_items(&x.f.maker, matched.reconciliation.id, 0, 101)
            .await
            .is_err()
    );
    // A manifest period cannot be rebound to a different immutable report range.
    file.range.from_date -= Duration::days(1);
    let other =
        x.f.f
            .create_report(&x.f.maker, Uuid::new_v4(), file.range)
            .await?
            .report
            .id;
    assert!(
        x.group(other, set.set_id)
            .await
            .err()
            .unwrap()
            .is::<bank_finance::reconciliation::ReconciliationInputError>()
    );
    drop(offline);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn group_reconciliation_owns_cases_after_lost_ack_and_wraps_beyond_fifty_conflicting_scopes()
-> Result<()> {
    use bank_controls::cases::{CaseKind, Cases, NewCase};
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let report = x.report(true).await?;
    let cutoff = Utc::now();
    let mut files = vec![];
    for _ in 0..2 {
        let rows = (0..50)
            .map(|_| SettlementRow {
                line_id: Uuid::new_v4(),
                payment_id: Uuid::new_v4(),
                evidence_id: Uuid::new_v4(),
                provider_version: 1,
                principal_minor: "1".into(),
                provider_fee_minor: "0".into(),
                currency_code: "TST".into(),
                currency_scale: 2,
                business_date: Utc::now().date_naive(),
            })
            .collect();
        let mut f = x.file(rows);
        f.generated_at = cutoff;
        x.publish_file(&f).await?;
        files.push(f.file_id);
    }
    let set = x.set(SetKind::Outgoing, files, cutoff);
    x.publish_set(&set).await?;
    let run = x.group(report, set.set_id).await?;
    assert_eq!(run.reconciliation.exception_count, 100);
    let pending=sqlx::query("SELECT i.id,r.operation_id FROM finance.group_reconciliation_issues i JOIN finance.group_reconciliation_items r ON r.run_id=i.run_id AND r.ordinal=i.ordinal WHERE i.run_id=$1 AND i.case_id IS NULL ORDER BY i.id LIMIT 52").bind(run.reconciliation.id).fetch_all(&x.f.db.migrator).await?;
    assert_eq!(pending.len(), 52);
    let mut service = actor("gl-export");
    service.issuer = "urn:piggybank:finance".into();
    service.client_id = "bank-worker".into();
    service.trust = TrustContext::Service;
    let cases = Cases::new(x.f.f.controls.pool.clone());
    let first: Uuid = pending[0].get("id");
    let operation: Uuid = pending[0].get("operation_id");
    let expected = cases
        .ensure(
            &service,
            NewCase {
                kind: CaseKind::Finance,
                origin_id: first,
                resource_id: operation,
                evidence_refs: vec![run.reconciliation.id, operation, first],
                customer: None,
            },
        )
        .await?;
    for row in pending.iter().skip(1).take(50) {
        cases
            .ensure(
                &service,
                NewCase {
                    kind: CaseKind::Finance,
                    origin_id: row.get("id"),
                    resource_id: Uuid::new_v4(),
                    evidence_refs: vec![],
                    customer: None,
                },
            )
            .await?;
    }
    for _ in 0..8 {
        x.f.f.recover_group_reconciliation_cases().await?;
    }
    let saved: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.group_reconciliation_issues WHERE id=$1")
            .bind(first)
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert_eq!(saved, Some(expected));
    let later: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.group_reconciliation_issues WHERE id=$1")
            .bind(pending[51].get::<Uuid, _>("id"))
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(later.is_some());
    let bad: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.group_reconciliation_issues WHERE id=$1")
            .bind(pending[1].get::<Uuid, _>("id"))
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(bad.is_none());
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn group_reconciliation_accepts_one_thousand_provider_rows_and_rejects_truncated_source_unions()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let report = x.report(true).await?;
    let cutoff = Utc::now();
    let mut files = vec![];
    for _ in 0..10 {
        let rows = (0..100)
            .map(|_| SettlementRow {
                line_id: Uuid::new_v4(),
                payment_id: Uuid::new_v4(),
                evidence_id: Uuid::new_v4(),
                provider_version: 1,
                principal_minor: "1".into(),
                provider_fee_minor: "0".into(),
                currency_code: "TST".into(),
                currency_scale: 2,
                business_date: Utc::now().date_naive(),
            })
            .collect();
        let mut f = x.file(rows);
        f.generated_at = cutoff;
        assert_eq!(x.publish_file(&f).await?, reqwest::StatusCode::CREATED);
        files.push(f.file_id);
    }
    let set = x.set(SetKind::Outgoing, files, cutoff);
    x.publish_set(&set).await?;
    let state = x.money_state().await?;
    let run = x.group(report, set.set_id).await?;
    assert_eq!(run.reconciliation.provider_row_count, 1000);
    assert_eq!(run.reconciliation.record_count, 1000);
    assert_eq!(run.reconciliation.exception_count, 1000);
    assert_eq!(run.reconciliation.provider_totals[0].principal, "1000");
    assert_eq!(state, x.money_state().await?);
    let page =
        x.f.f
            .group_reconciliation_items(&x.f.checker, run.reconciliation.id, 900, 100)
            .await?;
    assert_eq!(page.items.len(), 100);
    assert!(page.next_after.is_none());
    assert_eq!(page.total, 1000);
    let r = x.instruction(false, 17).await?;
    x.settle(&r, true).await?;
    let fresh = x.report(true).await?;
    let current = x.money_state().await?;
    assert!(
        x.group(fresh, set.set_id)
            .await
            .err()
            .unwrap()
            .is::<bank_core::finance_snapshot::SnapshotLimitExceeded>()
    );
    assert_eq!(current, x.money_state().await?);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM finance.group_reconciliation_runs")
            .fetch_one(&x.f.db.migrator)
            .await?,
        1
    );
    assert_eq!(
        x.f.f
            .group_reconciliation_detail(&x.f.checker, run.reconciliation.id)
            .await?
            .digest,
        run.digest
    );
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn group_reconciliation_binds_public_row_claims_to_signed_members_even_after_artifact_rehash()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let e = x.credit(123, None, false).await?;
    x.receive_credit(&e, true, true).await?;
    let report = x.report(true).await?;
    let file = x.credit_file(&[e]);
    x.publish_credit_file(&file).await?;
    let set = x.set(SetKind::Credits, vec![file.file_id], file.generated_at);
    x.publish_set(&set).await?;
    let run = x.group(report, set.set_id).await?;
    assert_eq!(run.reconciliation.completeness, "COMPLETE");
    let state = x.money_state().await?;
    let mut item: Value = sqlx::query_scalar(
        "SELECT document FROM finance.group_reconciliation_items WHERE run_id=$1",
    )
    .bind(run.reconciliation.id)
    .fetch_one(&x.f.db.migrator)
    .await?;
    item["credit"]["providerRows"][0]["principalMinor"] = json!("124");
    let mut summary: Value =
        sqlx::query_scalar("SELECT document FROM finance.group_reconciliation_runs WHERE id=$1")
            .bind(run.reconciliation.id)
            .fetch_one(&x.f.db.migrator)
            .await?;
    summary["itemsDigest"] = json!(fingerprint(&vec![item.clone()])?);
    // Simulate a trusted storage repair that incorrectly rehashes a changed public claim.
    let mut tx = x.f.db.migrator.begin().await?;
    sqlx::query("ALTER TABLE finance.group_reconciliation_items DISABLE TRIGGER immutable_group_reconciliation_items").execute(&mut *tx).await?;
    sqlx::query("ALTER TABLE finance.group_reconciliation_runs DISABLE TRIGGER immutable_group_reconciliation_runs").execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE finance.group_reconciliation_items SET document=$2,digest=$3 WHERE run_id=$1",
    )
    .bind(run.reconciliation.id)
    .bind(&item)
    .bind(fingerprint(&item)?)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE finance.group_reconciliation_runs SET document=$2,digest=$3 WHERE id=$1")
        .bind(run.reconciliation.id)
        .bind(&summary)
        .bind(fingerprint(&summary)?)
        .execute(&mut *tx)
        .await?;
    sqlx::query("ALTER TABLE finance.group_reconciliation_items ENABLE TRIGGER immutable_group_reconciliation_items").execute(&mut *tx).await?;
    sqlx::query("ALTER TABLE finance.group_reconciliation_runs ENABLE TRIGGER immutable_group_reconciliation_runs").execute(&mut *tx).await?;
    tx.commit().await?;
    assert!(
        x.f.f
            .group_reconciliation_detail(&x.f.maker, run.reconciliation.id)
            .await
            .is_err()
    );
    assert_eq!(state, x.money_state().await?);
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn group_reconciliation_rejects_outgoing_claims_after_closed_cutoff_and_preserves_exact_boundary()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let r = x.instruction(true, 17).await?;
    let row = x.settle(&r, true).await?.unwrap();
    let report = x.report(true).await?;
    let body: Value = sqlx::query_scalar("SELECT document FROM simulator.rail_events WHERE id=$1")
        .bind(row.evidence_id)
        .fetch_one(&x.f.db.migrator)
        .await?;
    let event: bank_rails::RailEvidence = serde_json::from_value(body)?;
    let state = x.money_state().await?;
    let mut f = x.file(vec![row]);
    f.generated_at = event.occurred_at - Duration::microseconds(1);
    x.publish_file(&f).await?;
    let set = x.set(SetKind::Outgoing, vec![f.file_id], f.generated_at);
    x.publish_set(&set).await?;
    let run = x.group(report, set.set_id).await?;
    assert_eq!(run.reconciliation.exception_count, 1);
    assert!(
        run.issues
            .iter()
            .any(|i| i.code == "PROVIDER_EVENT_AFTER_FILE_CUTOFF" && i.case_id.is_some())
    );
    let mut later = f.clone();
    later.file_id = Uuid::new_v4();
    later.generated_at = event.occurred_at;
    x.publish_file(&later).await?;
    let exact = x.set(SetKind::Outgoing, vec![later.file_id], later.generated_at);
    x.publish_set(&exact).await?;
    assert_eq!(
        x.group(report, exact.set_id)
            .await?
            .reconciliation
            .completeness,
        "COMPLETE"
    );
    let empty = SettlementFile {
        file_id: Uuid::new_v4(),
        rows: vec![],
        ..f
    };
    x.publish_file(&empty).await?;
    let late = x.set(SetKind::Outgoing, vec![empty.file_id], empty.generated_at);
    x.publish_set(&late).await?;
    let observed = x.group(report, late.set_id).await?;
    assert_eq!(observed.reconciliation.timing_count, 1);
    assert_eq!(observed.reconciliation.exception_count, 0);
    assert!(
        observed
            .issues
            .iter()
            .any(|i| i.code == "SETTLED_AFTER_FILE")
    );
    assert_eq!(state, x.money_state().await?);
    drop(server);
    x.f.close().await
}
