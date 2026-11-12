use bank_adapters::object_store::FilesystemStore;
use bank_controls::{
    audit::{AuditLog, AuditRecord},
    evidence::{BlobStore, EvidenceService},
};
use bank_platform::{
    database::Module,
    domain::{ActorContext, TrustContext},
    testing::TestDatabase,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn protected_audit_and_evidence_recover_on_real_postgresql() -> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Controls).await?;
    let foreign = db.module(Module::Platform).await?;
    let actor = ActorContext {
        issuer: "urn:synthetic:customers".into(),
        subject: Uuid::new_v4().to_string(),
        trust: TrustContext::Customer,
        client_id: "test".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let audit = AuditLog::new(pool.clone());
    let id = Uuid::new_v4();
    let record = AuditRecord::new(&actor, "test.event", Uuid::new_v4(), Uuid::new_v4());
    let digest = audit.append(id, "test", &record).await?;
    assert_eq!(digest, audit.append(id, "test", &record).await?);
    let mut changed = record.clone();
    changed.action = "test.changed".into();
    assert!(audit.append(id, "test", &changed).await.is_err());
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let audit = audit.clone();
        let actor = actor.clone();
        tasks.push(tokio::spawn(async move {
            audit
                .append(
                    Uuid::new_v4(),
                    "parallel",
                    &AuditRecord::new(&actor, "test.parallel", Uuid::new_v4(), Uuid::new_v4()),
                )
                .await
        }));
    }
    for task in tasks {
        task.await??;
    }
    assert!(audit.verify("test").await? && audit.verify("parallel").await?);
    assert!(
        sqlx::query("UPDATE controls.audit_events SET payload='{}'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("TRUNCATE controls.audit_events CASCADE")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE controls.audit_heads SET sequence=0")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE controls.audit_events SET payload='{}'")
            .execute(&db.migrator)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM controls.audit_events")
            .fetch_all(&foreign)
            .await
            .is_err()
    );
    let root = std::env::temp_dir().join(format!("piggybank-evidence-{}", Uuid::new_v4()));
    let store = Arc::new(FilesystemStore::new(&root)?);
    let evidence = EvidenceService::new(pool.clone(), store.clone());
    let reference = evidence
        .put(
            &actor,
            "SYNTHETIC",
            b"restricted synthetic evidence".to_vec(),
            Uuid::new_v4(),
        )
        .await?;
    assert_eq!(
        evidence.get(&actor, reference.id, Uuid::new_v4()).await?,
        b"restricted synthetic evidence"
    );
    let mut other = actor.clone();
    other.subject = Uuid::new_v4().to_string();
    assert!(
        evidence
            .get(&other, reference.id, Uuid::new_v4())
            .await
            .is_err()
    );
    assert!(store.write_new(reference.id, b"replacement").is_err());
    assert!(Uuid::parse_str("../../etc/passwd").is_err());
    std::fs::write(root.join(format!("{}.bin", reference.id)), b"corrupted")?;
    assert!(
        evidence
            .get(&actor, reference.id, Uuid::new_v4())
            .await
            .is_err()
    );
    #[cfg(unix)]
    {
        let symlink_id = Uuid::new_v4();
        std::os::unix::fs::symlink("/etc/passwd", root.join(format!("{symlink_id}.bin")))?;
        assert!(store.read_bounded(symlink_id, 1024).is_err());
    }
    let recovery = Uuid::new_v4();
    let data = b"durable orphaned commit";
    store.write_new(recovery, data)?;
    sqlx::query("INSERT INTO controls.evidence_objects(id,owner_issuer,owner_subject,purpose,digest,size,state,created_at) VALUES($1,$2,$3,'SYNTHETIC',$4,$5,'STAGED',now()-interval '2 minutes')")
        .bind(recovery).bind(&actor.issuer).bind(&actor.subject).bind(format!("{:x}",Sha256::digest(data))).bind(data.len() as i64).execute(&pool).await?;
    assert_eq!(evidence.recover_staged().await?, 1);
    assert_eq!(evidence.get(&actor, recovery, Uuid::new_v4()).await?, data);
    let orphan = Uuid::new_v4();
    store.write_new(orphan, b"inventory only")?;
    assert!(evidence.unreferenced().await?.contains(&orphan));
    assert!(audit.verify("evidence").await?);
    pool.close().await;
    foreign.close().await;
    db.close().await?;
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[tokio::test]
async fn case_information_and_restricted_staff_retrieval_preserve_ownership() -> anyhow::Result<()>
{
    use bank_controls::cases::{CaseKind, Cases, Disposition, NewCase};
    let db = TestDatabase::new().await?;
    let pool = db.module(Module::Controls).await?;
    let cases = Cases::new(pool.clone());
    let path = std::env::temp_dir().join(format!(
        "piggybank_case_evidence_{}",
        Uuid::new_v4().simple()
    ));
    let evidence = EvidenceService::new(pool.clone(), Arc::new(FilesystemStore::new(&path)?));
    let actor = |subject: &str, trust| ActorContext {
        issuer: "urn:case-evidence-fixture".into(),
        subject: subject.into(),
        trust,
        client_id: "fixture".into(),
        person_id: None,
        represented_party: None,
        roles: vec![],
    };
    let customer = actor("customer", TrustContext::Customer);
    let foreign = actor("other-customer", TrustContext::Customer);
    let reviewer = actor("reviewer", TrustContext::Workforce);
    let unrelated = actor("unrelated-reviewer", TrustContext::Workforce);
    for who in [&reviewer, &unrelated] {
        let person = Uuid::new_v4();
        sqlx::query("INSERT INTO controls.person_links VALUES($1,$2,$3)")
            .bind(&who.issuer)
            .bind(&who.subject)
            .bind(person)
            .execute(&db.migrator)
            .await?;
        sqlx::query("INSERT INTO controls.role_grants(id,person_id,role,valid_until) VALUES($1,$2,'case_reviewer',now()+interval '1 day')").bind(Uuid::new_v4()).bind(person).execute(&db.migrator).await?;
        if who.subject == reviewer.subject {
            sqlx::query("INSERT INTO controls.case_queue_members(queue,person_id,capability) VALUES('support',$1,'REVIEW')").bind(person).execute(&db.migrator).await?;
        }
    }
    let source = NewCase {
        kind: CaseKind::Support,
        origin_id: Uuid::new_v4(),
        resource_id: Uuid::new_v4(),
        evidence_refs: vec![],
        customer: Some((customer.issuer.clone(), customer.subject.clone())),
    };
    let case = cases.ensure(&customer, source).await?;
    cases.claim(&reviewer, case).await?;
    cases
        .request_information(&reviewer, case, "IDENTITY_EVIDENCE_REQUIRED")
        .await?;
    let key = Uuid::new_v4();
    let content = b"Restricted synthetic supporting information".to_vec();
    assert!(
        cases
            .provide_information(&foreign, case, key, content.clone(), &evidence)
            .await
            .is_err()
    );
    let id = cases
        .provide_information(&customer, case, key, content.clone(), &evidence)
        .await?;
    assert_eq!(
        id,
        cases
            .provide_information(&customer, case, key, content.clone(), &evidence)
            .await?
    );
    assert!(
        cases
            .provide_information(&customer, case, key, b"changed".to_vec(), &evidence)
            .await
            .is_err()
    );
    assert_eq!(cases.detail(&reviewer, case).await?.state, "OPEN");
    assert_eq!(
        evidence.get_for_case(&reviewer, case, id, &cases).await?,
        content
    );
    assert!(
        evidence
            .get_for_case(&unrelated, case, id, &cases)
            .await
            .is_err()
    );
    assert!(
        evidence
            .get_for_case(&customer, case, id, &cases)
            .await
            .is_err()
    );
    assert!(
        evidence
            .get_for_case(&reviewer, case, Uuid::new_v4(), &cases)
            .await
            .is_err()
    );
    let proposal = cases
        .propose(&reviewer, case, Disposition::ResolveService)
        .await?;
    cases.withdraw(&reviewer, case).await?;
    assert_eq!(cases.detail(&reviewer, case).await?.state, "OPEN");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM controls.case_proposals WHERE id=$1")
            .bind(proposal)
            .fetch_one(&pool)
            .await?,
        "WITHDRAWN"
    );
    assert!(
        AuditLog::new(pool.clone()).verify("cases").await?
            && AuditLog::new(pool.clone()).verify("evidence").await?
    );
    pool.close().await;
    db.close().await?;
    std::fs::remove_dir_all(path)?;
    Ok(())
}
