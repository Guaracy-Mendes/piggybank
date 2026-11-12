use super::*;
use bank_core::{
    accounts::{ActivationEvidence, OpeningPolicy, OpeningRequest},
    payments::{ExecutionPermit, InternalInstruction, PaymentPolicy},
    products::ProductVersion,
};
use bank_finance::{
    reconciliation::ReconciliationInput,
    settlement::{FEE_POLICY, PROVIDER, SettlementFile, SettlementRow},
};
use bank_platform::{authenticity::SignedDocument, domain::PaymentDetails};
use bank_rails::{RailEvidence, RailInstruction, RailStatus};
use futures_util::future::BoxFuture;
use serde_json::Value;
struct TrustedOpening;
impl OpeningPolicy for TrustedOpening {
    fn validate<'a>(&'a self, r: &'a OpeningRequest) -> BoxFuture<'a, Result<ActivationEvidence>> {
        Box::pin(async move {
            Ok(ActivationEvidence {
                application_id: r.application_id,
                party_id: r.party_id,
                product_version_id: r.product_version_id,
                acceptance_id: r.acceptance_id,
                controls_revision: "trusted-finance-acceptance".into(),
                expires_at: Utc::now() + Duration::minutes(10),
            })
        })
    }
}
struct TrustedPayment;
impl PaymentPolicy for TrustedPayment {
    fn validate<'a>(
        &'a self,
        r: &'a InternalInstruction,
    ) -> BoxFuture<'a, Result<ExecutionPermit>> {
        Box::pin(async move {
            Ok(ExecutionPermit {
                request_digest: fingerprint(r)?,
                valid_until: Utc::now() + Duration::minutes(3),
            })
        })
    }
}
struct PaymentFixture {
    f: Fixture,
    product: ProductVersion,
    from: AccountId,
    to: AccountId,
}
impl PaymentFixture {
    async fn new() -> Result<Self> {
        let f = Fixture::new().await?;
        f.publish(&f.chart).await?;
        for (who, role) in [
            (&f.maker, "product_admin"),
            (&f.checker, "product_approver"),
        ] {
            let person = f.f.controls.person(who).await?;
            sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&f.db.migrator).await?;
        }
        let product = ProductVersion {
            id: Uuid::new_v4(),
            code: "FINANCE_RECONCILIATION".into(),
            version: 1,
            name: "Synthetic reconciliation pricing".into(),
            label: "SYNTHETIC".into(),
            effective_from: Utc::now().date_naive() - Duration::days(5),
            effective_until: Utc::now().date_naive() + Duration::days(30),
            minimum_age: 18,
            payment_limit: money(50_000_000_000_000_000),
            daily_limit: money(100_000_000_000_000_000),
            flat_fee: money(7),
            fee_basis_points: 0,
            chart_version: "synthetic-chart-v1".into(),
        };
        let proposal =
            f.f.controls
                .propose_change(
                    &f.maker,
                    ChangeKind::Product,
                    product.id,
                    &product,
                    Utc::now() + Duration::minutes(10),
                )
                .await?;
        f.f.controls.approve_change(&f.checker, proposal).await?;
        f.core
            .publish_product(&f.f.controls, &f.maker, proposal, &product)
            .await?;
        let mut accounts = vec![];
        for name in ["customer-a", "customer-b"] {
            let mut who = actor(name);
            who.trust = TrustContext::Customer;
            who.client_id = "customer-web".into();
            let application_id = Uuid::new_v4();
            let party = f
                .core
                .ensure_party(&who, application_id, &fingerprint(&name)?)
                .await?;
            let request = OpeningRequest {
                application_id,
                party_id: party,
                product_version_id: product.id,
                acceptance_id: Uuid::new_v4(),
            };
            let account = f.core.stage_account(&request).await?;
            f.core.activate(account, &request, &TrustedOpening).await?;
            accounts.push(account);
        }
        let clearing = AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?);
        f.core
            .fund_synthetic(
                Uuid::new_v4(),
                accounts[0],
                clearing,
                money(100_000_000_000_000_000),
            )
            .await?;
        Ok(Self {
            f,
            product,
            from: accounts[0],
            to: accounts[1],
        })
    }
    async fn instruction(&self, external: bool, principal: i64) -> Result<InternalInstruction> {
        let beneficiary = Uuid::new_v4();
        let quote = Uuid::new_v4();
        let id = Uuid::new_v4();
        let rail = if external { "SIMULATED" } else { "INTERNAL" };
        let payee = if external {
            AccountId(Uuid::parse_str("4f1a780b-3e31-48ef-a294-000000000003")?)
        } else {
            self.to
        };
        let reference = if external {
            "SIM00000000000000000000000000000001".into()
        } else {
            format!("PB{}", self.to.0.simple()).to_uppercase()
        };
        let instruction = InternalInstruction {
            operation_id: id,
            details: PaymentDetails {
                payer_account_id: self.from,
                beneficiary_id: beneficiary.to_string(),
                amount: money(principal),
                fee: money(7),
                requested_date: Utc::now().date_naive(),
                rail: rail.into(),
                quote_id: quote.to_string(),
            },
            payee_account_id: payee,
            product_version_id: self.product.id,
            beneficiary_version: 1,
            rail_policy_version_id: external.then(Uuid::new_v4),
        };
        // Insert only orchestration fixture metadata; all money below uses guarded Core commands.
        sqlx::query("INSERT INTO payments.beneficiaries(id,issuer,subject,request_key,label,receiving_reference,payee_account_id,state,fingerprint,rail) VALUES($1,'urn:finance-customer','customer-a',$2,'Synthetic recipient',$3,$4,'VERIFIED','synthetic-fixture',$5)").bind(beneficiary).bind(Uuid::new_v4()).bind(reference).bind(payee.0).bind(rail).execute(&self.f.db.migrator).await?;
        sqlx::query("INSERT INTO payments.quotes(id,issuer,subject,beneficiary_id,beneficiary_version,instruction,details_digest,total_minor,expires_at) VALUES($1,'urn:finance-customer','customer-a',$2,1,$3,$4,$5,now()+interval '3 minutes')").bind(quote).bind(beneficiary).bind(serde_json::to_value(&instruction)?).bind(fingerprint(&instruction)?).bind(principal+7).execute(&self.f.db.migrator).await?;
        sqlx::query("INSERT INTO payments.intents(id,issuer,subject,client_id,request_key,quote_id,actor_context,state,confirmed_at) VALUES($1,'urn:finance-customer','customer-a','customer-web',$2,$3,$4,'ACCEPTED',now())").bind(id).bind(Uuid::new_v4()).bind(quote).bind(json!(actor("customer-a"))).execute(&self.f.db.migrator).await?;
        Ok(instruction)
    }
    async fn settle(&self, r: &InternalInstruction, ack: bool) -> Result<Option<SettlementRow>> {
        let mut row = None;
        let batch = if r.details.rail == "INTERNAL" {
            self.f
                .core
                .execute_internal_payment(r, &TrustedPayment)
                .await?
        } else {
            let reserved = self
                .f
                .core
                .reserve_external_payment(r, &TrustedPayment)
                .await?;
            self.f
                .core
                .mark_external_submitted(r, Utc::now() + Duration::minutes(2))
                .await?;
            let work = RailInstruction {
                instruction_id: r.operation_id,
                payment_id: r.operation_id,
                beneficiary_reference: "SIM00000000000000000000000000000001".into(),
                amount: r.details.amount,
                requested_date: r.details.requested_date,
                input_digest: fingerprint(r)?,
                rail: "SIMULATED".into(),
                authority_until: reserved.authority_until,
            };
            sqlx::query("INSERT INTO payments.rail_submissions(payment_id,instruction_id,document,fingerprint,attempted_at,attempts) VALUES($1,$1,$2,$3,now(),1)").bind(r.operation_id).bind(serde_json::to_value(&work)?).bind(fingerprint(&work)?).execute(&self.f.db.migrator).await?;
            bank_rails::RailClient::new(&self.f.base)?
                .submit(&work)
                .await?;
            let response = reqwest::Client::new()
                .post(format!("{}/admin/rail-events", self.f.base))
                .header("X-Rail-Key", bank_rails::signing_key())
                .json(&json!({"instructionId":r.operation_id,"status":"SETTLED"}))
                .send()
                .await?
                .error_for_status()?;
            let packet: SignedDocument = response.json().await?;
            let e: RailEvidence = serde_json::from_value(packet.body.clone())?;
            assert_eq!(e.status, RailStatus::Settled);
            let finality = self.f.core.finalize_external(r, &packet).await?;
            sqlx::query("INSERT INTO payments.rail_receipts VALUES($1,$2,$3,$4,$5,now())")
                .bind(e.evidence_id)
                .bind(r.operation_id)
                .bind(e.version)
                .bind(fingerprint(&e)?)
                .bind(serde_json::to_value(packet)?)
                .execute(&self.f.db.migrator)
                .await?;
            row = Some(SettlementRow {
                line_id: Uuid::new_v4(),
                payment_id: r.operation_id,
                evidence_id: e.evidence_id,
                provider_version: e.version,
                principal_minor: r.details.amount.minor_units().to_string(),
                provider_fee_minor: "0".into(),
                currency_code: "TST".into(),
                currency_scale: 2,
                business_date: e.settlement_date.unwrap(),
            });
            finality.batch_id.unwrap()
        };
        if ack {
            sqlx::query("UPDATE payments.intents SET state='SETTLED',batch_id=$2,version=version+1,reason_code='POSTED',updated_at=now() WHERE id=$1").bind(r.operation_id).bind(batch).execute(&self.f.db.migrator).await?;
        }
        Ok(row)
    }
    fn file(&self, rows: Vec<SettlementRow>) -> SettlementFile {
        SettlementFile {
            file_id: Uuid::new_v4(),
            format_version: 1,
            provider: PROVIDER.into(),
            fee_policy: FEE_POLICY.into(),
            range: range(),
            generated_at: Utc::now(),
            rows,
        }
    }
    async fn publish_file(&self, file: &SettlementFile) -> Result<reqwest::StatusCode> {
        Ok(reqwest::Client::new()
            .post(format!("{}/admin/settlement-files", self.f.base))
            .header("X-Finance-Key", bank_finance::protocol::signing_key())
            .json(file)
            .send()
            .await?
            .status())
    }
    async fn report(&self, export: bool) -> Result<Uuid> {
        if export {
            let ids = self
                .f
                .core
                .snapshot(range())
                .await?
                .batches
                .into_iter()
                .map(|s| s.journal.id)
                .collect::<Vec<_>>();
            for id in ids {
                let e = self.f.f.stage_export(id).await?;
                assert!(matches!(
                    self.f.f.process_export(e.id).await?,
                    Progress::Done { .. }
                ));
            }
        }
        Ok(self
            .f
            .f
            .create_report(&self.f.maker, Uuid::new_v4(), range())
            .await?
            .report
            .id)
    }
    async fn money_state(&self) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT jsonb_build_object('batches',(SELECT count(*) FROM core.journal_batches),'executions',(SELECT count(*) FROM core.payment_executions),'accounts',(SELECT jsonb_agg(jsonb_build_array(id,booked,held) ORDER BY id) FROM core.ledger_accounts),'holds',(SELECT count(*) FROM core.holds))").fetch_one(&self.f.db.migrator).await?)
    }
}
#[tokio::test]
async fn reconciliation_exact_multiway_lineage_replay_and_current_authority_never_post_money()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let internal = x.instruction(false, 9_007_199_254_740_993).await?;
    x.settle(&internal, true).await?;
    let external = x.instruction(true, 12345).await?;
    let row = x.settle(&external, true).await?.unwrap();
    let report = x.report(true).await?;
    let file = x.file(vec![row]);
    assert_eq!(x.publish_file(&file).await?, reqwest::StatusCode::CREATED);
    assert_eq!(x.publish_file(&file).await?, reqwest::StatusCode::OK);
    let before = x.money_state().await?;
    let key = Uuid::new_v4();
    let input = ReconciliationInput {
        report_id: report,
        file_id: file.file_id,
    };
    let (a, b) = tokio::join!(
        x.f.f.create_reconciliation(&x.f.maker, key, input.clone()),
        x.f.f.create_reconciliation(&x.f.maker, key, input.clone())
    );
    let a = a?;
    let b = b?;
    assert_eq!(a.reconciliation.id, b.reconciliation.id);
    assert_eq!(a.digest, b.digest);
    assert_eq!(a.reconciliation.matched_count, 2);
    assert_eq!(a.reconciliation.completeness, "COMPLETE");
    assert_eq!(a.reconciliation.core_bank_fee_total, "14");
    assert_eq!(a.reconciliation.matched_principal_total, "9007199254753338");
    assert!(a.issues.is_empty());
    let page =
        x.f.f
            .reconciliation_items(&x.f.checker, a.reconciliation.id, 0, 1)
            .await?;
    assert_eq!(page.total, 2);
    assert_eq!(page.next_after, Some(1));
    assert!(
        x.f.f
            .reconciliation_items(&x.f.maker, a.reconciliation.id, 0, 101)
            .await
            .is_err()
    );
    let mut changed = input;
    changed.file_id = Uuid::new_v4();
    let error =
        x.f.f
            .create_reconciliation(&x.f.maker, key, changed)
            .await
            .err()
            .unwrap();
    assert!(
        error
            .downcast_ref::<bank_finance::reconciliation::ReconciliationKeyConflict>()
            .is_some()
    );
    let stranger = actor("unauthorized");
    assert!(
        x.f.f
            .reconciliation_detail(&stranger, a.reconciliation.id)
            .await
            .is_err()
    );
    assert!(
        x.f.f
            .create_reconciliation(
                &x.f.checker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: file.file_id
                }
            )
            .await
            .is_err()
    );
    let person = x.f.f.controls.person(&x.f.maker).await?;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=now() WHERE person_id=$1 AND queue='finance-operations'").bind(person.0).execute(&x.f.db.migrator).await?;
    assert!(
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                key,
                ReconciliationInput {
                    report_id: report,
                    file_id: file.file_id
                }
            )
            .await
            .is_err()
    );
    assert_eq!(before, x.money_state().await?);
    for statement in [
        "UPDATE finance.reconciliation_runs SET digest='bad'",
        "DELETE FROM finance.reconciliation_items",
        "TRUNCATE finance.reconciliation_runs CASCADE",
        "UPDATE simulator.settlement_files SET digest='bad'",
    ] {
        assert!(
            sqlx::query(statement)
                .execute(&x.f.db.migrator)
                .await
                .is_err(),
            "{statement}"
        );
    }
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn reconciliation_authentic_file_differences_are_partitioned_owned_and_frozen() -> Result<()>
{
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let r = x.instruction(true, 5000).await?;
    let row = x.settle(&r, true).await?.unwrap();
    let report = x.report(true).await?;
    let before = x.money_state().await?;
    let mut variants = vec![];
    variants.push(("MISSING_PROVIDER_SETTLEMENT", vec![]));
    variants.push(("DUPLICATE_PROVIDER_LINE", vec![row.clone(), row.clone()]));
    let mut bad = row.clone();
    bad.currency_code = "USD".into();
    variants.push(("PROVIDER_CURRENCY_MISMATCH", vec![bad]));
    let mut bad = row.clone();
    bad.currency_scale = 3;
    variants.push(("PROVIDER_CURRENCY_MISMATCH", vec![bad]));
    let mut bad = row.clone();
    bad.principal_minor = "5001".into();
    variants.push(("PROVIDER_PRINCIPAL_MISMATCH", vec![bad]));
    let mut bad = row.clone();
    bad.provider_fee_minor = "9".into();
    variants.push(("PROVIDER_FEE_UNEXPECTED", vec![bad]));
    let mut bad = row.clone();
    bad.business_date += Duration::days(1);
    variants.push(("PROVIDER_DATE_MISMATCH", vec![bad]));
    let mut bad = row.clone();
    bad.evidence_id = Uuid::new_v4();
    variants.push(("PROVIDER_SETTLEMENT_EVIDENCE_MISMATCH", vec![bad]));
    let mut bad = row.clone();
    bad.payment_id = Uuid::new_v4();
    variants.push(("MISSING_PAYMENT_INSTRUCTION", vec![bad]));
    for (code, rows) in variants {
        let file = x.file(rows);
        assert_eq!(x.publish_file(&file).await?, reqwest::StatusCode::CREATED);
        let run =
            x.f.f
                .create_reconciliation(
                    &x.f.maker,
                    Uuid::new_v4(),
                    ReconciliationInput {
                        report_id: report,
                        file_id: file.file_id,
                    },
                )
                .await?;
        let items =
            x.f.f
                .reconciliation_items(&x.f.maker, run.reconciliation.id, 0, 100)
                .await?;
        assert!(
            items
                .items
                .iter()
                .any(|i| i.issue_codes.iter().any(|c| c == code)),
            "Missing {code}"
        );
        assert!(run.issues.iter().any(|i| i.code == code));
        assert!(run.issues.iter().all(|i| i.case_id.is_some()));
        assert_ne!(run.reconciliation.completeness, "COMPLETE");
        assert_eq!(
            x.f.f
                .reconciliation_detail(&x.f.maker, run.reconciliation.id)
                .await?
                .digest,
            run.digest
        );
    }
    let good = x.file(vec![row]);
    x.publish_file(&good).await?;
    let mut changed = good.clone();
    changed.rows[0].provider_fee_minor = "3".into();
    assert_eq!(
        x.publish_file(&changed).await?,
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(before, x.money_state().await?);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM finance.reconciliation_issues i JOIN controls.cases c ON c.origin_id=i.id").fetch_one(&x.f.db.migrator).await?;
    assert!(count > 8);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn reconciliation_distinguishes_frozen_snapshot_timing_and_unbooked_states() -> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let pending = x.instruction(true, 1000).await?;
    let early = x.report(true).await?;
    let empty = x.file(vec![]);
    x.publish_file(&empty).await?;
    let run =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: early,
                    file_id: empty.file_id,
                },
            )
            .await?;
    assert_eq!(run.reconciliation.pending_count, 1);
    assert!(run.issues.is_empty());
    let row = x.settle(&pending, true).await?.unwrap();
    let complete = x.file(vec![row]);
    x.publish_file(&complete).await?;
    let late =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: early,
                    file_id: complete.file_id,
                },
            )
            .await?;
    assert_eq!(late.reconciliation.timing_count, 1);
    assert!(late.issues.iter().any(|i| i.code == "CORE_AFTER_REPORT"));
    let new_report = x.report(true).await?;
    let matched =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: new_report,
                    file_id: complete.file_id,
                },
            )
            .await?;
    assert_eq!(matched.reconciliation.completeness, "COMPLETE");
    assert_eq!(
        x.f.f
            .reconciliation_detail(&x.f.maker, run.reconciliation.id)
            .await?
            .digest,
        run.digest
    );
    let another = x.instruction(false, 100).await?;
    x.settle(&another, false).await?;
    let ack_report = x.report(true).await?;
    let ack =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: ack_report,
                    file_id: complete.file_id,
                },
            )
            .await?;
    assert_eq!(ack.reconciliation.timing_count, 1);
    assert!(!ack.issues.iter().any(|i| i.code == "PAYMENT_ACK_TIMING"));
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn reconciliation_rejects_untrusted_or_unbounded_files_and_detects_private_archive_damage()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let r = x.instruction(true, 500).await?;
    let row = x.settle(&r, true).await?.unwrap();
    let report = x.report(true).await?;
    let file = x.file(vec![row.clone()]);
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{}/admin/settlement-files", x.f.base))
            .json(&file)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let packet = SignedDocument::sign(
        b"wrong-authentication-key-over-thirty-two-bytes",
        &bank_finance::settlement::SettlementQuery::new(file.file_id),
    )?;
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{}/settlement-files/inquiry", x.f.base))
            .json(&packet)
            .send()
            .await?
            .status(),
        reqwest::StatusCode::UNAUTHORIZED
    );
    let mut bad = file.clone();
    bad.rows[0].principal_minor = "0500".into();
    assert_eq!(
        x.publish_file(&bad).await?,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut big = file.clone();
    big.rows = vec![row; 101];
    assert_eq!(
        x.publish_file(&big).await?,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut period = file.clone();
    period.file_id = Uuid::new_v4();
    period.range.from_date -= Duration::days(1);
    x.publish_file(&period).await?;
    assert!(
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: period.file_id
                }
            )
            .await
            .is_err()
    );
    x.publish_file(&file).await?;
    let run =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: file.file_id,
                },
            )
            .await?;
    // A trusted fault fixture models storage corruption beyond normal immutable SQL privileges.
    sqlx::query(
        "ALTER TABLE finance.reconciliation_runs DISABLE TRIGGER immutable_reconciliation_runs",
    )
    .execute(&x.f.db.migrator)
    .await?;
    sqlx::query("UPDATE finance.reconciliation_runs SET payments_snapshot=jsonb_set(payments_snapshot,'{asOf}',to_jsonb('2000-01-01T00:00:00Z'::text)) WHERE id=$1").bind(run.reconciliation.id).execute(&x.f.db.migrator).await?;
    sqlx::query(
        "ALTER TABLE finance.reconciliation_runs ENABLE TRIGGER immutable_reconciliation_runs",
    )
    .execute(&x.f.db.migrator)
    .await?;
    assert!(
        x.f.f
            .reconciliation_detail(&x.f.maker, run.reconciliation.id)
            .await
            .is_err()
    );
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn reconciliation_gl_timing_and_core_bank_fee_corruption_remain_distinct_from_provider_fees()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let r = x.instruction(false, 3000).await?;
    x.settle(&r, true).await?;
    let early = x.report(false).await?;
    let file = x.file(vec![]);
    x.publish_file(&file).await?;
    let first =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: early,
                    file_id: file.file_id,
                },
            )
            .await?;
    assert_eq!(first.reconciliation.timing_count, 1);
    assert!(first.issues.is_empty());
    let page =
        x.f.f
            .reconciliation_items(&x.f.maker, first.reconciliation.id, 0, 10)
            .await?;
    assert_eq!(page.items[0].issue_codes, vec!["GL_TIMING_WINDOW"]);
    let complete = x.report(true).await?;
    let before = x.money_state().await?;
    // Trusted storage-fault fixture: change a receipt's fee, never its immutable journal legs.
    sqlx::query("ALTER TABLE core.payment_executions DISABLE TRIGGER immutable_payment_executions")
        .execute(&x.f.db.migrator)
        .await?;
    sqlx::query("UPDATE core.payment_executions SET fee=8 WHERE operation_id=$1")
        .bind(r.operation_id)
        .execute(&x.f.db.migrator)
        .await?;
    sqlx::query("ALTER TABLE core.payment_executions ENABLE TRIGGER immutable_payment_executions")
        .execute(&x.f.db.migrator)
        .await?;
    let wrong =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: complete,
                    file_id: file.file_id,
                },
            )
            .await?;
    assert_eq!(wrong.reconciliation.exception_count, 1);
    assert_eq!(wrong.reconciliation.core_bank_fee_total, "8");
    assert!(
        wrong
            .issues
            .iter()
            .any(|i| i.code == "CORE_BANK_FEE_MISMATCH" && i.case_id.is_some())
    );
    assert!(wrong.reconciliation.provider_totals.is_empty());
    assert_eq!(before, x.money_state().await?);
    assert_eq!(
        x.f.f
            .reconciliation_detail(&x.f.maker, first.reconciliation.id)
            .await?
            .digest,
        first.digest
    );
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn reconciliation_case_cursor_recovers_lost_ack_and_passes_fifty_bad_scopes() -> Result<()> {
    use bank_controls::cases::{CaseKind, Cases, NewCase};
    use sqlx::Row;
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let report = x.report(true).await?;
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
    let file = x.file(rows);
    x.publish_file(&file).await?;
    let run =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: file.file_id,
                },
            )
            .await?;
    assert_eq!(run.reconciliation.exception_count, 100);
    assert!(run.issues.iter().filter(|i| i.case_id.is_none()).count() > 52);
    let pending=sqlx::query("SELECT i.id,r.payment_id FROM finance.reconciliation_issues i JOIN finance.reconciliation_items r ON r.run_id=i.run_id AND r.ordinal=i.ordinal WHERE i.run_id=$1 AND i.case_id IS NULL ORDER BY i.id LIMIT 52").bind(run.reconciliation.id).fetch_all(&x.f.db.migrator).await?;
    let mut service = actor("gl-export");
    service.issuer = "urn:piggybank:finance".into();
    service.client_id = "bank-worker".into();
    service.trust = TrustContext::Service;
    let cases = Cases::new(x.f.f.controls.pool.clone());
    let ack: Uuid = pending[0].get("id");
    let payment: Uuid = pending[0].get("payment_id");
    // Controls committed its case, then Finance's acknowledgment was lost.
    let expected = cases
        .ensure(
            &service,
            NewCase {
                kind: CaseKind::Finance,
                origin_id: ack,
                resource_id: payment,
                evidence_refs: vec![run.reconciliation.id, payment, ack],
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
    let later: Uuid = pending[51].get("id");
    x.f.f.recover_reconciliation_cases().await?;
    x.f.f.recover_reconciliation_cases().await?;
    let actual: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.reconciliation_issues WHERE id=$1")
            .bind(ack)
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert_eq!(actual, Some(expected));
    let actual: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.reconciliation_issues WHERE id=$1")
            .bind(later)
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(actual.is_some());
    let first_bad: Uuid = pending[1].get("id");
    let actual: Option<Uuid> =
        sqlx::query_scalar("SELECT case_id FROM finance.reconciliation_issues WHERE id=$1")
            .bind(first_bad)
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(actual.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM controls.cases WHERE origin_id=$1")
            .bind(ack)
            .fetch_one(&x.f.db.migrator)
            .await?,
        1
    );
    drop(server);
    x.f.close().await
}

#[tokio::test]
async fn reconciliation_late_commit_is_timing_even_when_transaction_timestamp_predates_report()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let r = x.instruction(false, 2000).await?;
    let mut tx = x.f.core.pool.begin().await?;
    let batch: Uuid =
        sqlx::query_scalar("SELECT batch_id FROM core.execute_internal_payment($1,$2,$3)")
            .bind(serde_json::to_value(&r)?)
            .bind(fingerprint(&r)?)
            .bind(Utc::now() + Duration::minutes(3))
            .fetch_one(&mut *tx)
            .await?;
    // The native transaction remains uncommitted while the report sees the previous ledger state.
    let report = x.report(true).await?;
    let original = x.f.f.report_detail(&x.f.maker, report).await?;
    tx.commit().await?;
    let created: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM core.payment_executions WHERE operation_id=$1")
            .bind(r.operation_id)
            .fetch_one(&x.f.db.migrator)
            .await?;
    assert!(created < original.report.core_as_of);
    sqlx::query("UPDATE payments.intents SET state='SETTLED',batch_id=$2,version=version+1,updated_at=now() WHERE id=$1").bind(r.operation_id).bind(batch).execute(&x.f.db.migrator).await?;
    let file = x.file(vec![]);
    x.publish_file(&file).await?;
    let observed =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: file.file_id,
                },
            )
            .await?;
    assert_eq!(observed.reconciliation.timing_count, 1);
    assert_eq!(observed.reconciliation.exception_count, 0);
    assert!(
        observed
            .issues
            .iter()
            .any(|i| i.code == "CORE_AFTER_REPORT")
    );
    assert!(
        !observed
            .issues
            .iter()
            .any(|i| i.code == "MISSING_CORE_JOURNAL")
    );
    assert_eq!(
        x.f.f.report_detail(&x.f.maker, report).await?.digest,
        original.digest
    );
    drop(server);
    x.f.close().await
}

struct CommitBetweenSources {
    core: Core,
    migrator: sqlx::PgPool,
    instruction: InternalInstruction,
    first: std::sync::atomic::AtomicBool,
}
impl FinanceRecords for CommitBetweenSources {
    fn credit_accounting<'a>(
        &'a self,
        range: bank_core::finance_snapshot::ReportRange,
        batches: &'a [Uuid],
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<bank_core::credit_accounting::CreditAccountingSnapshot>> {
        Box::pin(self.core.credit_accounting(range, batches, ids))
    }
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
        operations: &'a [Uuid],
    ) -> BoxFuture<'a, Result<bank_core::payment_accounting::PaymentAccountingSnapshot>> {
        Box::pin(async move {
            let snapshot = self
                .core
                .payment_accounting(range, batches, operations)
                .await?;
            if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                // A genuine native commit and its workflow ACK occur after the first Core read.
                let batch = self
                    .core
                    .execute_internal_payment(&self.instruction, &TrustedPayment)
                    .await?;
                sqlx::query("UPDATE payments.intents SET state='SETTLED',batch_id=$2,version=version+1,updated_at=now() WHERE id=$1").bind(self.instruction.operation_id).bind(batch).execute(&self.migrator).await?;
            }
            Ok(snapshot)
        })
    }
}
#[tokio::test]
async fn reconciliation_confirms_native_receipts_after_payments_snapshot_without_false_missing_money()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let instruction = x.instruction(false, 1500).await?;
    let report = x.report(true).await?;
    let file = x.file(vec![]);
    x.publish_file(&file).await?;
    let mut finance = x.f.f.clone();
    finance.core = Arc::new(CommitBetweenSources {
        core: x.f.core.clone(),
        migrator: x.f.db.migrator.clone(),
        instruction,
        first: std::sync::atomic::AtomicBool::new(true),
    });
    let run = finance
        .create_reconciliation(
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
    assert!(run.issues.iter().any(|i| i.code == "CORE_AFTER_REPORT"));
    assert!(!run.issues.iter().any(|i| i.code == "MISSING_CORE_PAYMENT"));
    assert_eq!(run.reconciliation.core_principal_total, "1500");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.payment_executions")
            .fetch_one(&x.f.db.migrator)
            .await?,
        1
    );
    drop(finance);
    drop(server);
    x.f.close().await
}
#[tokio::test]
async fn reconciliation_file_cutoff_uses_authenticated_provider_event_time_not_bank_processing_time()
-> Result<()> {
    let x = PaymentFixture::new().await?;
    let server = x.f.server()?;
    x.f.wait().await?;
    let r = x.instruction(true, 500).await?;
    let row = x.settle(&r, true).await?.unwrap();
    let report = x.report(true).await?;
    let document: Value =
        sqlx::query_scalar("SELECT document FROM simulator.rail_events WHERE id=$1")
            .bind(row.evidence_id)
            .fetch_one(&x.f.db.migrator)
            .await?;
    let event: RailEvidence = serde_json::from_value(document)?;
    let recorded: chrono::DateTime<Utc> = sqlx::query_scalar(
        "SELECT recorded_at FROM core.external_finalities WHERE operation_id=$1",
    )
    .bind(r.operation_id)
    .fetch_one(&x.f.db.migrator)
    .await?;
    assert!(recorded > event.occurred_at);
    let mut file = x.file(vec![]);
    file.generated_at = event.occurred_at;
    x.publish_file(&file).await?;
    let run =
        x.f.f
            .create_reconciliation(
                &x.f.maker,
                Uuid::new_v4(),
                ReconciliationInput {
                    report_id: report,
                    file_id: file.file_id,
                },
            )
            .await?;
    assert!(
        run.issues
            .iter()
            .any(|i| i.code == "MISSING_PROVIDER_SETTLEMENT")
    );
    assert!(!run.issues.iter().any(|i| i.code == "SETTLED_AFTER_FILE"));
    drop(server);
    x.f.close().await
}

#[path = "credit_reconciliation.rs"]
mod credit_reconciliation;
