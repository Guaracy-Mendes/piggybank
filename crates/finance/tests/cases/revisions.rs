use super::*;
async fn pair(
    f: &Fixture,
) -> Result<(
    bank_finance::reports::ReportDetail,
    bank_finance::reports::ReportDetail,
)> {
    f.source(1024).await?;
    let a = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    f.source(11).await?;
    let b = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    Ok((a, b))
}
fn input(a: Uuid, b: Uuid) -> bank_finance::revisions::RevisionInput {
    bank_finance::revisions::RevisionInput {
        original_report_id: a,
        replacement_report_id: b,
        reason: "Include the newly booked Core source and preserve the earlier snapshot".into(),
    }
}
async fn grant(f: &Fixture, who: &ActorContext, role: &str, cap: &str) -> Result<()> {
    let person = f.f.controls.person(who).await?;
    sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,$3,now()+interval '1 day')").bind(Uuid::new_v4()).bind(person.0).bind(role).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('finance-operations',$1,$2) ON CONFLICT(queue,person_id,capability) DO UPDATE SET revoked_at=NULL").bind(person.0).bind(cap).execute(&f.db.migrator).await?;
    Ok(())
}
#[tokio::test]
async fn independent_exact_scope_and_recovery_of_registration_and_publication_acknowledgments()
-> Result<()> {
    let f = Fixture::new().await?;
    let (a, b) = pair(&f).await?;
    let input = input(a.report.id, b.report.id);
    let key = Uuid::new_v4();
    let (x, y) = tokio::join!(
        f.f.propose_report_revision(&f.maker, key, &input),
        f.f.propose_report_revision(&f.maker, key, &input)
    );
    let staged = x?;
    assert_eq!(staged.id, y?.id);
    assert_eq!(staged.state, "STAGED");
    assert_eq!(staged.details.debit_delta, "11");
    assert_eq!(staged.details.source_count_delta, 1);
    let mut changed = input.clone();
    changed.reason = "Changed approved purpose".into();
    assert!(
        f.f.propose_report_revision(&f.maker, key, &changed)
            .await
            .is_err()
    );
    // Controls committed registration, but Finance never received the acknowledgment.
    f.f.controls
        .propose_change_id(
            &f.maker,
            staged.id,
            ChangeKind::FinanceReport,
            a.report.id,
            &staged.details,
            staged.expires_at,
        )
        .await?;
    assert!(
        f.f.report_revision(&f.maker, staged.id)
            .await?
            .registered_at
            .is_none()
    );
    f.f.recover_report_revisions().await?;
    f.f.recover_report_revisions().await?;
    assert!(
        f.f.report_revision(&f.maker, staged.id)
            .await?
            .registered_at
            .is_some()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM controls.change_proposals WHERE id=$1")
            .bind(staged.id)
            .fetch_one(&f.db.migrator)
            .await?,
        1
    );
    let alias = actor("maker-approver-alias");
    f.f.controls
        .link_person(&alias, f.f.controls.person(&f.maker).await?)
        .await?;
    grant(&f, &alias, "finance_approver", "APPROVE").await?;
    assert!(
        f.f.approve_report_revision(&alias, staged.id, &staged.details_digest)
            .await
            .is_err()
    );
    assert!(
        f.f.publish_report_revision(&f.maker, staged.id, &staged.details_digest)
            .await
            .is_err()
    );
    assert!(
        f.f.approve_report_revision(&f.checker, staged.id, &"0".repeat(64))
            .await
            .is_err()
    );
    let approved =
        f.f.approve_report_revision(&f.checker, staged.id, &staged.details_digest)
            .await?;
    assert_eq!(approved.state, "APPROVED");
    assert!(
        f.f.controls
            .approved_change(
                &f.maker,
                staged.id,
                ChangeKind::Correction,
                a.report.id,
                &staged.details
            )
            .await
            .is_err()
    );
    let published =
        f.f.publish_report_revision(&f.maker, staged.id, &staged.details_digest)
            .await?;
    assert_eq!(published.state, "PUBLISHED");
    assert_ne!(published.maker_person, published.checker_person.unwrap());
    let replay =
        f.f.publish_report_revision(&alias, staged.id, &staged.details_digest)
            .await?;
    assert_eq!(replay.published_at, published.published_at);
    assert_eq!(
        f.f.propose_report_revision(&f.maker, key, &input)
            .await?
            .state,
        "PUBLISHED"
    );
    let original = f.f.report_detail(&f.checker, a.report.id).await?;
    let replacement = f.f.report_detail(&f.checker, b.report.id).await?;
    assert_eq!(original.digest, a.digest);
    assert_eq!(replacement.digest, b.digest);
    assert_eq!(original.revisions.len(), 1);
    assert_eq!(replacement.revisions[0].proposal_id, staged.id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.journal_batches")
            .fetch_one(&f.db.migrator)
            .await?,
        2
    );
    assert!(
        sqlx::query("UPDATE finance.report_revision_requests SET details='{}'")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM finance.report_revisions")
            .execute(&f.f.pool)
            .await
            .is_err()
    );
    f.f.relay_audit().await?;
    f.close().await
}
#[tokio::test]
async fn publication_prevents_forks_cycles_cross_periods_and_other_makers() -> Result<()> {
    let f = Fixture::new().await?;
    let (a, b) = pair(&f).await?;
    let c = f.f.create_report(&f.maker, Uuid::new_v4(), range()).await?;
    let p =
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(a.report.id, b.report.id))
            .await?;
    let q =
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(a.report.id, c.report.id))
            .await?;
    for r in [&p, &q] {
        f.f.approve_report_revision(&f.checker, r.id, &r.details_digest)
            .await?;
    }
    let other = actor("other-finance-maker");
    f.f.controls.link_person(&other, PartyId::new()).await?;
    grant(&f, &other, "finance_adjuster", "REVIEW").await?;
    assert!(
        f.f.publish_report_revision(&other, p.id, &p.details_digest)
            .await
            .is_err()
    );
    let (x, y) = tokio::join!(
        f.f.publish_report_revision(&f.maker, p.id, &p.details_digest),
        f.f.publish_report_revision(&f.maker, q.id, &q.details_digest)
    );
    assert_eq!(usize::from(x.is_ok()) + usize::from(y.is_ok()), 1);
    assert_eq!(
        f.f.report_detail(&f.maker, a.report.id)
            .await?
            .revisions
            .len(),
        1
    );
    assert!(
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(c.report.id, a.report.id))
            .await
            .is_err()
    );
    let mut different = range();
    different.from_date -= Duration::days(1);
    let d =
        f.f.create_report(&f.maker, Uuid::new_v4(), different)
            .await?;
    assert!(
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(b.report.id, d.report.id))
            .await
            .is_err()
    );
    assert!(
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(b.report.id, b.report.id))
            .await
            .is_err()
    );
    f.close().await
}
#[tokio::test]
async fn revocation_and_expired_proofs_cannot_publish_new_links() -> Result<()> {
    let f = Fixture::new().await?;
    let (a, b) = pair(&f).await?;
    let p =
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(a.report.id, b.report.id))
            .await?;
    f.f.approve_report_revision(&f.checker, p.id, &p.details_digest)
        .await?;
    let checker = f.f.controls.person(&f.checker).await?;
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=now() WHERE person_id=$1 AND queue='finance-operations' AND capability='APPROVE'").bind(checker.0).execute(&f.db.migrator).await?;
    assert!(
        f.f.publish_report_revision(&f.maker, p.id, &p.details_digest)
            .await
            .is_err()
    );
    sqlx::query("UPDATE controls.case_queue_members SET revoked_at=NULL WHERE person_id=$1 AND queue='finance-operations' AND capability='APPROVE'").bind(checker.0).execute(&f.db.migrator).await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='finance_approver'").bind(checker.0).execute(&f.db.migrator).await?;
    assert!(
        f.f.publish_report_revision(&f.maker, p.id, &p.details_digest)
            .await
            .is_err()
    );
    grant(&f, &f.checker, "finance_approver", "APPROVE").await?;
    let expired = Uuid::new_v4();
    sqlx::query("INSERT INTO finance.report_revision_requests(id,original_report_id,replacement_report_id,issuer,subject,client_id,request_key,actor_context,maker_person,details,details_digest,expires_at) SELECT $2,original_report_id,replacement_report_id,issuer,subject,client_id,$3,actor_context,maker_person,details,details_digest,now()-interval '1 minute' FROM finance.report_revision_requests WHERE id=$1").bind(p.id).bind(expired).bind(Uuid::new_v4()).execute(&f.db.migrator).await?;
    sqlx::query("INSERT INTO controls.change_proposals(id,kind,resource_id,digest,maker_person,checker_person,state,expires_at) SELECT id,'FINANCE_REPORT',original_report_id,details_digest,maker_person,$2,'APPROVED',expires_at FROM finance.report_revision_requests WHERE id=$1").bind(expired).bind(checker.0).execute(&f.db.migrator).await?;
    assert_eq!(
        f.f.report_revision(&f.maker, expired).await?.state,
        "EXPIRED"
    );
    assert!(
        f.f.publish_report_revision(&f.maker, expired, &p.details_digest)
            .await
            .is_err()
    );
    let maker = f.f.controls.person(&f.maker).await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='finance_adjuster'").bind(maker.0).execute(&f.db.migrator).await?;
    assert!(
        f.f.publish_report_revision(&f.maker, p.id, &p.details_digest)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM finance.report_revisions")
            .fetch_one(&f.f.pool)
            .await?,
        0
    );
    f.close().await
}
#[tokio::test]
async fn registration_pages_pass_revoked_requests_and_recover_later_work() -> Result<()> {
    let f = Fixture::new().await?;
    let (a, b) = pair(&f).await?;
    let p =
        f.f.propose_report_revision(&f.maker, Uuid::new_v4(), &input(a.report.id, b.report.id))
            .await?;
    let maker = f.f.controls.person(&f.maker).await?;
    let checker = f.f.controls.person(&f.checker).await?;
    for n in 1..=50 {
        sqlx::query("INSERT INTO finance.report_revision_requests(id,original_report_id,replacement_report_id,issuer,subject,client_id,request_key,actor_context,maker_person,details,details_digest,expires_at) SELECT $2,original_report_id,replacement_report_id,issuer,subject,client_id,$3,actor_context,maker_person,details,details_digest,expires_at FROM finance.report_revision_requests WHERE id=$1").bind(p.id).bind(Uuid::from_u128(n)).bind(Uuid::new_v4()).execute(&f.db.migrator).await?;
    }
    grant(&f, &f.checker, "finance_adjuster", "REVIEW").await?;
    let live = Uuid::from_u128(u128::MAX);
    sqlx::query("INSERT INTO finance.report_revision_requests(id,original_report_id,replacement_report_id,issuer,subject,client_id,request_key,actor_context,maker_person,details,details_digest,expires_at) SELECT $2,original_report_id,replacement_report_id,$3,$4,$5,$6,$7,$8,details,details_digest,expires_at FROM finance.report_revision_requests WHERE id=$1").bind(p.id).bind(live).bind(&f.checker.issuer).bind(&f.checker.subject).bind(&f.checker.client_id).bind(Uuid::new_v4()).bind(serde_json::to_value(&f.checker)?).bind(checker.0).execute(&f.db.migrator).await?;
    sqlx::query("UPDATE controls.role_grants SET revoked_at=now() WHERE person_id=$1 AND role='finance_adjuster'").bind(maker.0).execute(&f.db.migrator).await?;
    f.f.recover_report_revisions().await?;
    assert!(
        f.f.report_revision(&f.checker, live)
            .await?
            .registered_at
            .is_none()
    );
    f.f.recover_report_revisions().await?;
    assert!(
        f.f.report_revision(&f.checker, live)
            .await?
            .registered_at
            .is_some()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM controls.change_proposals WHERE kind='FINANCE_REPORT'"
        )
        .fetch_one(&f.db.migrator)
        .await?,
        1
    );
    f.close().await
}
