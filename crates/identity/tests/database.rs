//! UI-03 contact codes: the identity role reaches codes only through the SQL functions, codes are stored hashed,
//! delivered once to the simulated outbox, rate limited, attempt limited, single use and expiring.
use bank_platform::{database::Module, testing::TestDatabase};
use serde_json::Value;

async fn issue(pool: &sqlx::PgPool, subject: &str, code: &str) -> Result<Value, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT identity.contact_code_issue('urn:fixture',$1,'reggie@example.invalid',$2)",
    )
    .bind(subject)
    .bind(code)
    .fetch_one(pool)
    .await
}
async fn check(pool: &sqlx::PgPool, subject: &str, code: &str) -> anyhow::Result<Value> {
    Ok(
        sqlx::query_scalar("SELECT identity.contact_code_check('urn:fixture',$1,$2)")
            .bind(subject)
            .bind(code)
            .fetch_one(pool)
            .await?,
    )
}
/// Moves a subject's codes into the past, as if time had passed.
async fn age(db: &TestDatabase, subject: &str, by: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE identity.contact_codes SET created_at=created_at-$2::interval,expires_at=expires_at-$2::interval,resend_after=resend_after-$2::interval WHERE subject=$1")
        .bind(subject)
        .bind(by)
        .execute(&db.migrator)
        .await?;
    Ok(())
}

#[tokio::test]
async fn contact_codes_are_hashed_rate_limited_attempt_limited_single_use_and_expiring()
-> anyhow::Result<()> {
    let db = TestDatabase::new().await?;
    let identity = db.module(Module::Identity).await?;

    // The identity role has no direct table access.
    assert!(
        sqlx::query("SELECT * FROM identity.contact_codes")
            .execute(&identity)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM simulator.contact_outbox")
            .execute(&identity)
            .await
            .is_err()
    );
    assert!(
        issue(&identity, "s1", "12345").await.is_err(),
        "6 digits only"
    );

    // Issue: stored as a salted digest, delivered once to the simulated outbox.
    let status = issue(&identity, "s1", "123456").await?;
    assert_eq!(status["attemptsLeft"], 5);
    let (hash, id): (String, uuid::Uuid) =
        sqlx::query_as("SELECT code_hash,id FROM identity.contact_codes WHERE subject='s1'")
            .fetch_one(&db.migrator)
            .await?;
    assert_ne!(hash, "123456");
    assert_ne!(
        hash,
        format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(b"123456"))
    );
    assert_eq!(hash.len(), 64);
    let outbox: Vec<String> = sqlx::query_scalar(
        "SELECT body FROM simulator.contact_outbox WHERE destination='reggie@example.invalid'",
    )
    .fetch_all(&db.migrator)
    .await?;
    assert_eq!(outbox.len(), 1);
    assert!(outbox[0].contains("123456") && outbox[0].contains("simulated"));
    let _ = id;

    // Resend gap of 45 seconds.
    let soon = issue(&identity, "s1", "654321").await.unwrap_err();
    assert!(soon.to_string().contains("RESEND_TOO_SOON"));

    // Misses use attempts; the fifth miss locks the code, and even the right code is then refused.
    for left in [4, 3, 2, 1] {
        let r = check(&identity, "s1", "000000").await?;
        assert_eq!(
            (r["result"].as_str(), r["attemptsLeft"].as_i64()),
            (Some("MISMATCH"), Some(left))
        );
    }
    assert_eq!(check(&identity, "s1", "000000").await?["result"], "LOCKED");
    assert_eq!(check(&identity, "s1", "123456").await?["result"], "LOCKED");

    // A new code supersedes the locked one; a match verifies once and cannot be replayed.
    age(&db, "s1", "46 seconds").await?;
    issue(&identity, "s1", "222222").await?;
    assert_eq!(
        check(&identity, "s1", "123456").await?["result"],
        "MISMATCH",
        "old code superseded"
    );
    let ok = check(&identity, "s1", "222222").await?;
    assert_eq!(
        (ok["result"].as_str(), ok["destination"].as_str()),
        (Some("VERIFIED"), Some("reggie@example.invalid"))
    );
    assert_eq!(
        check(&identity, "s1", "222222").await?["result"],
        "EXPIRED",
        "single use"
    );

    // Expiry after 10 minutes.
    issue(&identity, "s2", "333333").await?;
    age(&db, "s2", "11 minutes").await?;
    assert_eq!(check(&identity, "s2", "333333").await?["result"], "EXPIRED");

    // At most 5 codes per subject per hour.
    for n in 0..5 {
        issue(&identity, "s3", &format!("44444{n}")).await?;
        age(&db, "s3", "46 seconds").await?;
    }
    let capped = issue(&identity, "s3", "555555").await.unwrap_err();
    assert!(capped.to_string().contains("TOO_MANY_CODES"));

    // Subjects are isolated.
    issue(&identity, "s4", "777777").await?;
    assert_eq!(check(&identity, "s5", "777777").await?["result"], "EXPIRED");
    Ok(())
}
