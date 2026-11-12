use super::*;
use bank_core::credit::CreditPolicy;
use bank_finance::credit_settlement::{CreditSettlementFile, CreditSettlementRow};
use bank_rails::{CreditEvidence, CreditKind};
struct TrustedCredit;
impl CreditPolicy for TrustedCredit {
    fn validate<'a>(&'a self, e: &'a CreditEvidence) -> BoxFuture<'a, Result<ExecutionPermit>> {
        Box::pin(async move {
            Ok(ExecutionPermit {
                request_digest: fingerprint(e)?,
                valid_until: Utc::now() + Duration::minutes(2),
            })
        })
    }
}
impl PaymentFixture {
    async fn credit(
        &self,
        principal: i64,
        original: Option<Uuid>,
        unknown: bool,
    ) -> Result<CreditEvidence> {
        let reference = if original.is_some() {
            None
        } else if unknown {
            Some(format!("PB{}", Uuid::new_v4().simple()).to_uppercase())
        } else {
            Some(
                sqlx::query_scalar("SELECT core.receiving_reference($1)")
                    .bind(self.to.0)
                    .fetch_one(&self.f.core.pool)
                    .await?,
            )
        };
        Ok(CreditEvidence {
            event_type: if original.is_some() {
                CreditKind::ReturnSettled
            } else {
                CreditKind::InboundSettled
            },
            movement_id: Uuid::new_v4(),
            evidence_id: Uuid::new_v4(),
            provider: bank_rails::PROVIDER.into(),
            rail: "SIMULATED".into(),
            original_payment_id: original,
            receiving_reference: reference,
            amount: money(principal),
            business_date: Utc::now().date_naive(),
            occurred_at: Utc::now(),
            version: 1,
            reason_code: "SYNTHETIC_CREDIT".into(),
        })
    }
    async fn receive_credit(&self, e: &CreditEvidence, allocate: bool, ack: bool) -> Result<()> {
        // Obtain the archived financial event from the independently persisted provider.
        let packet: SignedDocument = reqwest::Client::new()
            .post(format!("{}/admin/rail-credits", self.f.base))
            .header("X-Rail-Key", bank_rails::signing_key())
            .json(e)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let target = self.f.core.credit_destination(e).await?.map(|(a, _)| a.0);
        sqlx::query("INSERT INTO payments.credit_intents(movement_id,evidence_id,document,fingerprint,target_account_id) VALUES($1,$2,$3,$4,$5)").bind(e.movement_id).bind(e.evidence_id).bind(serde_json::to_value(&packet)?).bind(fingerprint(e)?).bind(target).execute(&self.f.db.migrator).await?;
        self.f.core.record_rail_credit(&packet).await?;
        if allocate {
            self.f.core.allocate_rail_credit(e, &TrustedCredit).await?;
        }
        if ack {
            sqlx::query(
                "UPDATE payments.credit_intents SET state=$2,updated_at=now() WHERE movement_id=$1",
            )
            .bind(e.movement_id)
            .bind(if allocate {
                "CREDITED"
            } else {
                "DESTINATION_RESTRICTED"
            })
            .execute(&self.f.db.migrator)
            .await?;
        }
        Ok(())
    }
    fn credit_file(&self, events: &[CreditEvidence]) -> CreditSettlementFile {
        CreditSettlementFile {
            file_id: Uuid::new_v4(),
            format_version: 1,
            provider: PROVIDER.into(),
            fee_policy: FEE_POLICY.into(),
            range: range(),
            generated_at: Utc::now(),
            rows: events
                .iter()
                .map(|e| CreditSettlementRow {
                    line_id: Uuid::new_v4(),
                    movement_id: e.movement_id,
                    event_type: e.event_type.clone(),
                    original_payment_id: e.original_payment_id,
                    receiving_reference: e.receiving_reference.clone(),
                    evidence_id: e.evidence_id,
                    provider_version: e.version,
                    principal_minor: e.amount.minor_units().to_string(),
                    provider_fee_minor: "0".into(),
                    currency_code: "TST".into(),
                    currency_scale: 2,
                    business_date: e.business_date,
                })
                .collect(),
        }
    }
    async fn publish_credit_file(&self, f: &CreditSettlementFile) -> Result<reqwest::StatusCode> {
        Ok(reqwest::Client::new()
            .post(format!("{}/admin/credit-settlement-files", self.f.base))
            .header("X-Finance-Key", bank_finance::protocol::signing_key())
            .json(f)
            .send()
            .await?
            .status())
    }
    async fn reconcile_credit(
        &self,
        report: Uuid,
        f: &CreditSettlementFile,
    ) -> Result<bank_finance::credit_reconciliation::CreditReconciliationDetail> {
        assert_eq!(
            self.publish_credit_file(f).await?,
            reqwest::StatusCode::CREATED
        );
        self.f
            .f
            .create_credit_reconciliation(
                &self.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: f.file_id,
                },
            )
            .await
    }
}
#[tokio::test]
async fn credit_reconciliation_exact_incoming_and_partial_returns_replay_and_principal_caps()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let original = x.instruction(true, 1000).await?;
    x.settle(&original, true).await?;
    let incoming = x.credit(9007199254740993, None, false).await?;
    let r1 = x.credit(400, Some(original.operation_id), false).await?;
    let r2 = x.credit(600, Some(original.operation_id), false).await?;
    for e in [&incoming, &r1, &r2] {
        x.receive_credit(e, true, true).await?;
    }
    let excess = x.credit(1, Some(original.operation_id), false).await?;
    let packet = SignedDocument::sign(bank_rails::signing_key().as_bytes(), &excess)?;
    assert!(x.f.core.record_rail_credit(&packet).await.is_err());
    let file = x.credit_file(&[incoming, r1, r2]);
    let report = x.report(true).await?;
    assert_eq!(
        x.publish_credit_file(&file).await?,
        reqwest::StatusCode::CREATED
    );
    assert_eq!(x.publish_credit_file(&file).await?, reqwest::StatusCode::OK);
    let state = x.money_state().await?;
    let key = Uuid::new_v4();
    let input = ReconciliationInput {
        report_id: report,
        file_id: file.file_id,
    };
    let (a, b) = tokio::try_join!(
        x.f.f
            .create_credit_reconciliation(&x.f.maker, key, input.clone()),
        x.f.f
            .create_credit_reconciliation(&x.f.maker, key, input.clone())
    )?;
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.reconciliation.matched_count, 3);
    assert_eq!(a.reconciliation.completeness, "COMPLETE");
    assert_eq!(a.reconciliation.core_principal_total, "9007199254741993");
    assert_eq!(
        a.reconciliation.core_allocated_principal_total,
        "9007199254741993"
    );
    assert_eq!(a.reconciliation.core_suspense_principal_total, "0");
    assert!(a.issues.is_empty());
    let page =
        x.f.f
            .credit_reconciliation_items(&x.f.checker, a.reconciliation.id, 0, 2)
            .await?;
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_after, Some(2));
    let next =
        x.f.f
            .credit_reconciliation_items(&x.f.maker, a.reconciliation.id, 2, 2)
            .await?;
    assert_eq!(next.items.len(), 1);
    assert!(next.next_after.is_none());
    assert_eq!(next.reconciliation_digest, a.digest);
    let all =
        x.f.f
            .credit_reconciliation_items(&x.f.maker, a.reconciliation.id, 0, 100)
            .await?;
    for item in all.items.iter().filter(|i| i.original_payment_id.is_some()) {
        assert_eq!(item.returned_principal_total.as_deref(), Some("1000"));
        assert!(item.original_batch_id.is_some());
        assert_eq!(item.journal_batch_ids.len(), 2);
        assert_eq!(item.gl_export_ids.len(), 2);
    }
    assert!(!serde_json::to_string(&all)?.contains("signature"));
    assert!(
        x.f.f
            .create_credit_reconciliation(&x.f.checker, Uuid::new_v4(), input.clone())
            .await
            .is_err()
    );
    assert!(
        x.f.f
            .create_credit_reconciliation(
                &x.f.maker,
                key,
                ReconciliationInput {
                    report_id: report,
                    file_id: Uuid::new_v4()
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
            .create_credit_reconciliation(&x.f.maker, key, input)
            .await
            .is_err()
    );
    assert_eq!(state, x.money_state().await?);
    assert!(
        sqlx::query("UPDATE finance.credit_reconciliation_runs SET digest='changed' WHERE id=$1")
            .bind(a.reconciliation.id)
            .execute(&x.f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM core.credit_movements")
            .fetch_all(&x.f.f.pool)
            .await
            .is_err()
    );
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn credit_reconciliation_owns_suspense_preserves_old_artifacts_and_handles_lost_ack()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let blocked = x.credit(501, None, false).await?;
    let unknown = x.credit(309, None, true).await?;
    x.receive_credit(&blocked, false, true).await?;
    x.receive_credit(&unknown, false, true).await?;
    let file = x.credit_file(&[blocked.clone(), unknown]);
    let report = x.report(true).await?;
    let observed = x.reconcile_credit(report, &file).await?;
    assert_eq!(observed.reconciliation.suspense_count, 2);
    assert_eq!(observed.reconciliation.core_suspense_principal_total, "810");
    assert_eq!(observed.reconciliation.core_allocated_principal_total, "0");
    assert_eq!(observed.reconciliation.completeness, "PARTIAL");
    assert_eq!(observed.issues.len(), 2);
    assert!(
        observed
            .issues
            .iter()
            .all(|i| i.code == "FUNDS_IN_SUSPENSE" && i.case_id.is_some())
    );
    x.f.core
        .allocate_rail_credit(&blocked, &TrustedCredit)
        .await?;
    let later_report = x.report(true).await?;
    let later = x
        .reconcile_credit(
            later_report,
            &CreditSettlementFile {
                file_id: Uuid::new_v4(),
                ..file.clone()
            },
        )
        .await?;
    assert_eq!(later.reconciliation.timing_count, 1);
    assert_eq!(later.reconciliation.suspense_count, 1);
    assert_eq!(later.reconciliation.core_allocated_principal_total, "501");
    assert!(!later.issues.iter().any(|i| i.code == "CREDIT_ACK_TIMING"));
    assert_eq!(
        x.f.f
            .credit_reconciliation_detail(&x.f.maker, observed.reconciliation.id)
            .await?
            .digest,
        observed.digest
    );
    sqlx::query(
        "UPDATE payments.credit_intents SET state='CREDITED',updated_at=now() WHERE movement_id=$1",
    )
    .bind(blocked.movement_id)
    .execute(&x.f.db.migrator)
    .await?;
    let fresh = x
        .reconcile_credit(
            later_report,
            &CreditSettlementFile {
                file_id: Uuid::new_v4(),
                ..file
            },
        )
        .await?;
    assert_eq!(fresh.reconciliation.matched_count, 1);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn credit_reconciliation_classifies_independent_file_differences_without_posting_money()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let e = x.credit(145, None, false).await?;
    x.receive_credit(&e, true, true).await?;
    let report = x.report(true).await?;
    let original = x.credit_file(&[e]);
    let baseline = x.reconcile_credit(report, &original).await?;
    let state = x.money_state().await?;
    for (name, mutated) in [
        (
            "PROVIDER_PRINCIPAL_MISMATCH",
            json!({"principalMinor":"146"}),
        ),
        ("PROVIDER_CURRENCY_MISMATCH", json!({"currencyCode":"USD"})),
        ("PROVIDER_CURRENCY_MISMATCH", json!({"currencyScale":3})),
        ("PROVIDER_FEE_UNEXPECTED", json!({"providerFeeMinor":"7"})),
        (
            "PROVIDER_CREDIT_SCOPE_MISMATCH",
            json!({"originalPaymentId":Uuid::new_v4()}),
        ),
        (
            "PROVIDER_CREDIT_SCOPE_MISMATCH",
            json!({"receivingReference":"PB00000000000000000000000000000000"}),
        ),
        (
            "PROVIDER_CREDIT_SCOPE_MISMATCH",
            json!({"eventType":"RETURN_SETTLED"}),
        ),
        (
            "PROVIDER_SETTLEMENT_EVIDENCE_MISMATCH",
            json!({"evidenceId":Uuid::new_v4()}),
        ),
        (
            "PROVIDER_SETTLEMENT_EVIDENCE_MISMATCH",
            json!({"providerVersion":2}),
        ),
        (
            "PROVIDER_DATE_MISMATCH",
            json!({"businessDate":(Utc::now().date_naive()-Duration::days(1)).to_string()}),
        ),
    ] {
        let mut f = original.clone();
        f.file_id = Uuid::new_v4();
        let mut row = serde_json::to_value(&f.rows[0])?;
        for (k, v) in mutated.as_object().unwrap() {
            row[k] = v.clone();
        }
        f.rows[0] = serde_json::from_value(row)?;
        let run = x.reconcile_credit(report, &f).await?;
        assert_eq!(run.reconciliation.exception_count, 1, "{name}");
        assert!(
            run.issues
                .iter()
                .any(|i| i.code == name && i.case_id.is_some()),
            "{name}"
        );
    }
    let mut duplicates = original.clone();
    duplicates.file_id = Uuid::new_v4();
    duplicates.rows.push(duplicates.rows[0].clone());
    let run = x.reconcile_credit(report, &duplicates).await?;
    for code in [
        "DUPLICATE_PROVIDER_MOVEMENT",
        "DUPLICATE_PROVIDER_LINE",
        "DUPLICATE_PROVIDER_EVIDENCE",
    ] {
        assert!(run.issues.iter().any(|i| i.code == code));
    }
    assert_eq!(run.reconciliation.provider_totals[0].principal, "290");
    let mut missing = original.clone();
    missing.file_id = Uuid::new_v4();
    missing.rows.clear();
    assert!(
        x.reconcile_credit(report, &missing)
            .await?
            .issues
            .iter()
            .any(|i| i.code == "MISSING_PROVIDER_SETTLEMENT")
    );
    assert_eq!(state, x.money_state().await?);
    assert_eq!(
        x.f.f
            .credit_reconciliation_detail(&x.f.maker, baseline.reconciliation.id)
            .await?
            .digest,
        baseline.digest
    );
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn credit_reconciliation_rejects_bad_scopes_bounds_unavailable_and_private_archive_tampering()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let e = x.credit(123, None, false).await?;
    x.receive_credit(&e, true, true).await?;
    let report = x.report(true).await?;
    let file = x.credit_file(&[e]);
    let absent =
        x.f.f
            .create_credit_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: Uuid::new_v4(),
                },
            )
            .await;
    assert!(
        absent
            .err()
            .unwrap()
            .is::<bank_finance::settlement::SettlementUnavailable>()
    );
    let mut wrong = file.clone();
    wrong.file_id = Uuid::new_v4();
    wrong.range.from_date -= Duration::days(1);
    assert!(
        x.reconcile_credit(report, &wrong)
            .await
            .err()
            .unwrap()
            .is::<bank_finance::reconciliation::ReconciliationInputError>()
    );
    let mut large = file.clone();
    large.rows = vec![file.rows[0].clone(); 101];
    assert_eq!(
        x.publish_credit_file(&large).await?,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let run = x.reconcile_credit(report, &file).await?;
    let mut conflict = file.clone();
    conflict.rows[0].provider_fee_minor = "1".into();
    assert_eq!(
        x.publish_credit_file(&conflict).await?,
        reqwest::StatusCode::CONFLICT
    );
    assert!(
        x.f.f
            .credit_reconciliation_items(&x.f.maker, run.reconciliation.id, -1, 2)
            .await
            .is_err()
    );
    assert!(
        x.f.f
            .credit_reconciliation_items(&x.f.maker, run.reconciliation.id, 0, 101)
            .await
            .is_err()
    );
    let mut bad = x.f.f.clone();
    bad.client = GlClient::new("http://127.0.0.1:1")?;
    assert_eq!(
        bad.create_credit_reconciliation(
            &x.f.maker,
            Uuid::new_v4(),
            ReconciliationInput {
                report_id: report,
                file_id: Uuid::new_v4()
            }
        )
        .await
        .err()
        .unwrap()
        .to_string(),
        "Authenticated settlement file unavailable"
    );
    // Trusted migration owner simulates storage damage. Runtime cannot update the artifact.
    sqlx::query("ALTER TABLE finance.credit_reconciliation_runs DISABLE TRIGGER immutable_credit_reconciliation_runs").execute(&x.f.db.migrator).await?;
    sqlx::query("UPDATE finance.credit_reconciliation_runs SET core_snapshot=jsonb_set(core_snapshot,'{facts,0,principal,minorUnits}','\"124\"') WHERE id=$1").bind(run.reconciliation.id).execute(&x.f.db.migrator).await?;
    sqlx::query("ALTER TABLE finance.credit_reconciliation_runs ENABLE TRIGGER immutable_credit_reconciliation_runs").execute(&x.f.db.migrator).await?;
    assert!(
        x.f.f
            .credit_reconciliation_detail(&x.f.maker, run.reconciliation.id)
            .await
            .is_err()
    );
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn credit_reconciliation_freezes_pending_then_observes_late_core_and_provider_cutoffs()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let report = x.report(true).await?;
    let e = x.credit(419, None, false).await?;
    let packet: SignedDocument = reqwest::Client::new()
        .post(format!("{}/admin/rail-credits", x.f.base))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&e)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let target = x.f.core.credit_destination(&e).await?.unwrap().0;
    sqlx::query("INSERT INTO payments.credit_intents(movement_id,evidence_id,document,fingerprint,target_account_id) VALUES($1,$2,$3,$4,$5)").bind(e.movement_id).bind(e.evidence_id).bind(serde_json::to_value(&packet)?).bind(fingerprint(&e)?).bind(target.0).execute(&x.f.db.migrator).await?;
    let file = x.credit_file(std::slice::from_ref(&e));
    let pending = x.reconcile_credit(report, &file).await?;
    assert_eq!(pending.reconciliation.pending_count, 1);
    assert_eq!(pending.reconciliation.core_principal_total, "0");
    assert!(pending.issues.is_empty());
    x.f.core.record_rail_credit(&packet).await?;
    x.f.core.allocate_rail_credit(&e, &TrustedCredit).await?;
    let late = x
        .reconcile_credit(
            report,
            &CreditSettlementFile {
                file_id: Uuid::new_v4(),
                ..file.clone()
            },
        )
        .await?;
    assert_eq!(late.reconciliation.timing_count, 1);
    assert_eq!(late.reconciliation.exception_count, 0);
    assert!(late.issues.iter().any(|i| i.code == "CORE_AFTER_REPORT"));
    assert_eq!(
        x.f.f
            .credit_reconciliation_detail(&x.f.maker, pending.reconciliation.id)
            .await?
            .digest,
        pending.digest
    );
    sqlx::query(
        "UPDATE payments.credit_intents SET state='CREDITED',updated_at=now() WHERE movement_id=$1",
    )
    .bind(e.movement_id)
    .execute(&x.f.db.migrator)
    .await?;
    let current = x.report(true).await?;
    let cutoff = CreditSettlementFile {
        file_id: Uuid::new_v4(),
        generated_at: e.occurred_at - Duration::milliseconds(1),
        rows: vec![],
        ..file.clone()
    };
    let early = x.reconcile_credit(current, &cutoff).await?;
    assert_eq!(early.reconciliation.timing_count, 1);
    assert!(early.issues.iter().any(|i| i.code == "SETTLED_AFTER_FILE"));
    let missing = x
        .reconcile_credit(
            current,
            &CreditSettlementFile {
                file_id: Uuid::new_v4(),
                generated_at: e.occurred_at,
                rows: vec![],
                ..file
            },
        )
        .await?;
    assert!(
        missing
            .issues
            .iter()
            .any(|i| i.code == "MISSING_PROVIDER_SETTLEMENT")
    );
    assert!(
        !missing
            .issues
            .iter()
            .any(|i| i.code == "SETTLED_AFTER_FILE")
    );
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn credit_reconciliation_preserves_provider_value_date_and_actual_bank_booking_date()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let mut e = x.credit(89, None, false).await?;
    e.business_date -= Duration::days(1);
    x.receive_credit(&e, true, true).await?;
    let report = x.report(true).await?;
    let file = x.credit_file(&[e]);
    let run = x.reconcile_credit(report, &file).await?;
    assert_eq!(run.reconciliation.timing_count, 1);
    assert_eq!(run.reconciliation.exception_count, 0);
    assert!(
        run.issues
            .iter()
            .any(|i| i.code == "CREDIT_BOOKING_DATE_DIFFERENCE")
    );
    let item =
        x.f.f
            .credit_reconciliation_items(&x.f.maker, run.reconciliation.id, 0, 1)
            .await?
            .items
            .remove(0);
    assert_ne!(
        item.suspense_business_date,
        Some(file.rows[0].business_date)
    );
    assert_eq!(item.suspense_business_date, item.allocation_business_date);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn credit_reconciliation_case_cursor_wraps_past_conflicts_and_recovers_lost_ack() -> Result<()>
{
    use bank_controls::cases::{CaseKind, Cases, NewCase};
    use sqlx::Row;
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let report = x.report(true).await?;
    let events = (0..100)
        .map(|_| CreditEvidence {
            event_type: CreditKind::InboundSettled,
            movement_id: Uuid::new_v4(),
            evidence_id: Uuid::new_v4(),
            provider: bank_rails::PROVIDER.into(),
            rail: "SIMULATED".into(),
            original_payment_id: None,
            receiving_reference: Some("PB00000000000000000000000000000000".into()),
            amount: money(1),
            business_date: Utc::now().date_naive(),
            occurred_at: Utc::now(),
            version: 1,
            reason_code: "SYNTHETIC_ORPHAN".into(),
        })
        .collect::<Vec<_>>();
    let run = x.reconcile_credit(report, &x.credit_file(&events)).await?;
    assert_eq!(run.reconciliation.exception_count, 100);
    let pending=sqlx::query("SELECT i.id,r.movement_id FROM finance.credit_reconciliation_issues i JOIN finance.credit_reconciliation_items r ON r.run_id=i.run_id AND r.ordinal=i.ordinal WHERE i.run_id=$1 AND i.case_id IS NULL ORDER BY i.id LIMIT 52").bind(run.reconciliation.id).fetch_all(&x.f.db.migrator).await?;
    assert_eq!(pending.len(), 52);
    let mut actor = actor("gl-export");
    actor.issuer = "urn:piggybank:finance".into();
    actor.client_id = "bank-worker".into();
    actor.trust = TrustContext::Service;
    let cases = Cases::new(x.f.f.controls.pool.clone());
    let first: Uuid = pending[0].get("id");
    let movement: Uuid = pending[0].get("movement_id");
    let expected = cases
        .ensure(
            &actor,
            NewCase {
                kind: CaseKind::Finance,
                origin_id: first,
                resource_id: movement,
                evidence_refs: vec![run.reconciliation.id, movement, first],
                customer: None,
            },
        )
        .await?;
    for r in pending.iter().skip(1).take(50) {
        cases
            .ensure(
                &actor,
                NewCase {
                    kind: CaseKind::Finance,
                    origin_id: r.get("id"),
                    resource_id: Uuid::new_v4(),
                    evidence_refs: vec![],
                    customer: None,
                },
            )
            .await?;
    }
    for _ in 0..8 {
        x.f.f.recover_credit_reconciliation_cases().await?;
    }
    let saved: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.credit_reconciliation_issues WHERE id=$1")
            .bind(first)
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert_eq!(saved, Some(expected));
    let later: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.credit_reconciliation_issues WHERE id=$1")
            .bind(pending[51].get::<Uuid, _>("id"))
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(later.is_some());
    let conflict: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.credit_reconciliation_issues WHERE id=$1")
            .bind(pending[1].get::<Uuid, _>("id"))
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(conflict.is_none());
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn credit_reconciliation_distinguishes_provider_period_from_booking_period_and_cutoff_conflicts()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let mut e = x.credit(93, None, false).await?;
    e.business_date -= Duration::days(1);
    x.receive_credit(&e, true, true).await?;
    x.report(true).await?;
    let today = bank_core::finance_snapshot::ReportRange {
        from_date: Utc::now().date_naive(),
        to_date: Utc::now().date_naive() + Duration::days(1),
    };
    let report =
        x.f.f
            .create_report(&x.f.maker, Uuid::new_v4(), today)
            .await?
            .report
            .id;
    let file = CreditSettlementFile {
        range: today,
        ..x.credit_file(&[])
    };
    let run = x.reconcile_credit(report, &file).await?;
    assert_eq!(run.reconciliation.timing_count, 1);
    assert_eq!(run.reconciliation.exception_count, 0);
    assert!(
        run.issues
            .iter()
            .any(|i| i.code == "CREDIT_VALUE_OUTSIDE_FILE_PERIOD")
    );
    assert!(
        !run.issues
            .iter()
            .any(|i| i.code == "MISSING_PROVIDER_SETTLEMENT")
    );
    let wide = x.report(true).await?;
    let mut impossible = x.credit_file(&[e.clone()]);
    impossible.generated_at = e.occurred_at - Duration::milliseconds(1);
    let conflict = x.reconcile_credit(wide, &impossible).await?;
    assert_eq!(conflict.reconciliation.exception_count, 1);
    assert!(
        conflict
            .issues
            .iter()
            .any(|i| i.code == "PROVIDER_EVENT_AFTER_FILE_CUTOFF")
    );
    drop(server);
    x.f.close().await
}
struct CreditCommitBetweenSources {
    core: Core,
    migrator: sqlx::PgPool,
    packet: SignedDocument,
    evidence: CreditEvidence,
    first: std::sync::atomic::AtomicBool,
}
impl FinanceRecords for CreditCommitBetweenSources {
    fn batch_ids<'a>(&'a self, after: Uuid) -> BoxFuture<'a, Result<Vec<Uuid>>> {
        self.core.batch_ids(after)
    }
    fn financial_batch<'a>(
        &'a self,
        id: Uuid,
    ) -> BoxFuture<'a, Result<bank_core::finance::FinancialBatch>> {
        self.core.financial_batch(id)
    }
    fn snapshot<'a>(
        &'a self,
        range: bank_core::finance_snapshot::ReportRange,
    ) -> BoxFuture<'a, Result<bank_core::finance_snapshot::FinancialSnapshot>> {
        self.core.snapshot(range)
    }
    fn positions<'a>(
        &'a self,
    ) -> BoxFuture<'a, Result<bank_core::finance_snapshot::PositionSnapshot>> {
        self.core.positions()
    }
    fn payment_accounting<'a>(
        &'a self,
        range: bank_core::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<bank_core::payment_accounting::PaymentAccountingSnapshot>> {
        Box::pin(self.core.payment_accounting(range, batches, ids))
    }
    fn credit_accounting<'a>(
        &'a self,
        range: bank_core::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<bank_core::credit_accounting::CreditAccountingSnapshot>> {
        Box::pin(async move {
            let snapshot = self.core.credit_accounting(range, batches, ids).await?;
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                self.core.record_rail_credit(&self.packet).await?;
                self.core
                    .allocate_rail_credit(&self.evidence, &TrustedCredit)
                    .await?;
                sqlx::query("UPDATE payments.credit_intents SET state='CREDITED',updated_at=now() WHERE movement_id=$1").bind(self.evidence.movement_id).execute(&self.migrator).await?;
            }
            Ok(snapshot)
        })
    }
}
#[tokio::test]
async fn credit_reconciliation_confirms_new_booking_with_prior_value_date_between_source_captures()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    x.report(true).await?;
    let today = bank_core::finance_snapshot::ReportRange {
        from_date: Utc::now().date_naive(),
        to_date: Utc::now().date_naive() + Duration::days(1),
    };
    let report =
        x.f.f
            .create_report(&x.f.maker, Uuid::new_v4(), today)
            .await?
            .report
            .id;
    let mut e = x.credit(91, None, false).await?;
    e.business_date -= Duration::days(1);
    let packet: SignedDocument = reqwest::Client::new()
        .post(format!("{}/admin/rail-credits", x.f.base))
        .header("X-Rail-Key", bank_rails::signing_key())
        .json(&e)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    sqlx::query("INSERT INTO payments.credit_intents(movement_id,evidence_id,document,fingerprint,target_account_id) VALUES($1,$2,$3,$4,$5)").bind(e.movement_id).bind(e.evidence_id).bind(serde_json::to_value(&packet)?).bind(fingerprint(&e)?).bind(x.to.0).execute(&x.f.db.migrator).await?;
    let file = CreditSettlementFile {
        range: today,
        ..x.credit_file(&[])
    };
    assert_eq!(
        x.publish_credit_file(&file).await?,
        reqwest::StatusCode::CREATED
    );
    let mut f = x.f.f.clone();
    f.core = Arc::new(CreditCommitBetweenSources {
        core: x.f.core.clone(),
        migrator: x.f.db.migrator.clone(),
        packet,
        evidence: e,
        first: std::sync::atomic::AtomicBool::new(true),
    });
    let run = f
        .create_credit_reconciliation(
            &x.f.maker,
            Uuid::new_v4(),
            ReconciliationInput {
                report_id: report,
                file_id: file.file_id,
            },
        )
        .await?;
    assert_eq!(run.reconciliation.timing_count, 1);
    assert_eq!(run.reconciliation.exception_count, 0);
    assert_eq!(run.reconciliation.core_allocated_principal_total, "91");
    for code in [
        "SOURCE_SNAPSHOT_TIMING",
        "CORE_AFTER_REPORT",
        "CREDIT_VALUE_OUTSIDE_FILE_PERIOD",
    ] {
        assert!(run.issues.iter().any(|i| i.code == code), "{code}");
    }
    assert!(
        !run.issues
            .iter()
            .any(|i| i.code == "MISSING_CREDIT_INSTRUCTION")
    );
    drop(f);
    drop(server);
    x.f.close().await
}

#[path = "group_reconciliation.rs"]
mod group_reconciliation;
