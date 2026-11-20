//! Owner-scoped consumer workspaces. All balances, orders and receipts are native Core facts.
use crate::Core;
use anyhow::{Result, ensure};
use bank_controls::entitlements::Entitlements;
use bank_platform::domain::{ActorContext, TrustContext};
use serde_json::Value;
use uuid::Uuid;
impl Core {
    async fn consumer_read_person(
        &self,
        c: &Entitlements,
        a: &ActorContext,
    ) -> Result<Option<Uuid>> {
        ensure!(
            a.trust == TrustContext::Customer,
            "Customer channel required"
        );
        Ok(sqlx::query_scalar(
            "SELECT person_id FROM controls.person_links WHERE issuer=$1 AND subject=$2",
        )
        .bind(&a.issuer)
        .bind(&a.subject)
        .fetch_optional(&c.pool)
        .await?)
    }
    pub async fn consumer_token_read(&self, secret: &str, scope: &str) -> Result<Value> {
        Ok(sqlx::query_scalar("SELECT core.consumer_token_read($1,$2)")
            .bind(secret)
            .bind(scope)
            .fetch_one(&self.pool)
            .await?)
    }
    async fn consumer_person(&self, controls: &Entitlements, actor: &ActorContext) -> Result<Uuid> {
        ensure!(
            actor.trust == TrustContext::Customer,
            "Customer channel required"
        );
        Ok(controls.person(actor).await?.0)
    }
    pub async fn consumer_snapshot(&self, c: &Entitlements, a: &ActorContext) -> Result<Value> {
        let Some(person) = self.consumer_read_person(c, a).await? else {
            let day = chrono::Utc::now().date_naive().to_string();
            return Ok(
                serde_json::json!({"needsOnboarding":true,"preferences":{},"accounts":[],"activity":[],"vaults":[],"cards":[],"budgets":[],"links":[],"bills":[],"notifications":[],"tokens":[],"requests":[],"groupWithdrawals":[],"statements":[],"roundups":null,"analytics":{"from":day,"to":day,"incomeMinor":"0","spentMinor":"0","categories":[],"merchants":[]}}),
            );
        };
        Ok(sqlx::query_scalar("SELECT core.consumer_snapshot($1,$2)")
            .bind(serde_json::to_value(a)?)
            .bind(person)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn wealth_snapshot(&self, c: &Entitlements, a: &ActorContext) -> Result<Value> {
        let Some(person) = self.consumer_read_person(c, a).await? else {
            let assets: Value=sqlx::query_scalar("SELECT jsonb_agg(jsonb_build_object('symbol',symbol,'name',name,'class',class,'sector',sector,'priceMinor',price_minor::text,'dividendBps',dividend_bps,'description',description,'watched',false,'drip',false,'alertMinor',NULL) ORDER BY symbol) FROM core.wealth_assets").fetch_one(&self.pool).await?;
            return Ok(
                serde_json::json!({"account":null,"assets":assets,"positions":[],"orders":[],"observations":[],"schedules":[],"dividends":[],"notice":"Complete account onboarding to use simulated investments. TST has no monetary value.","dataSource":"SYNTHETIC_PRICE_FIXTURE","quoteObservedAt":chrono::Utc::now()}),
            );
        };
        Ok(sqlx::query_scalar("SELECT core.wealth_snapshot($1,$2)")
            .bind(serde_json::to_value(a)?)
            .bind(person)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn consumer_command(
        &self,
        c: &Entitlements,
        a: &ActorContext,
        key: Uuid,
        action: &str,
        input: &Value,
    ) -> Result<Value> {
        let person = self.consumer_person(c, a).await?;
        let wealth = matches!(
            action,
            "BROKER_OPEN"
                | "BROKER_DEPOSIT"
                | "BROKER_WITHDRAW"
                | "TRADE_EXECUTE"
                | "ORDER_CANCEL"
                | "WATCHLIST"
                | "SCHEDULE_CREATE"
                | "SCHEDULE_UPDATE"
        );
        let query = if wealth {
            "SELECT core.wealth_command($1,$2,$3,$4,$5)"
        } else {
            "SELECT core.consumer_command($1,$2,$3,$4,$5)"
        };
        Ok(sqlx::query_scalar(query)
            .bind(serde_json::to_value(a)?)
            .bind(person)
            .bind(key)
            .bind(action)
            .bind(input)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn consumer_review(
        &self,
        c: &Entitlements,
        a: &ActorContext,
        action: &str,
        input: &Value,
    ) -> Result<Value> {
        let person = self.consumer_person(c, a).await?;
        Ok(
            sqlx::query_scalar("SELECT core.consumer_review($1,$2,$3,$4)")
                .bind(serde_json::to_value(a)?)
                .bind(person)
                .bind(action)
                .bind(input)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn consumer_authorization(
        &self,
        c: &Entitlements,
        a: &ActorContext,
        id: Uuid,
    ) -> Result<Value> {
        let person = self.consumer_person(c, a).await?;
        Ok(
            sqlx::query_scalar("SELECT core.consumer_authorization($1,$2,$3)")
                .bind(serde_json::to_value(a)?)
                .bind(person)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn wealth_quote(
        &self,
        c: &Entitlements,
        a: &ActorContext,
        input: &Value,
    ) -> Result<Value> {
        let person = self.consumer_person(c, a).await?;
        Ok(sqlx::query_scalar("SELECT core.wealth_quote($1,$2,$3)")
            .bind(serde_json::to_value(a)?)
            .bind(person)
            .bind(input)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn consumer_tick(&self) -> Result<i32> {
        Ok(sqlx::query_scalar("SELECT core.consumer_tick()")
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn consumer_statement(
        &self,
        c: &Entitlements,
        a: &ActorContext,
        id: Uuid,
    ) -> Result<Value> {
        let person = self.consumer_person(c, a).await?;
        Ok(
            sqlx::query_scalar("SELECT core.consumer_statement($1,$2,$3)")
                .bind(serde_json::to_value(a)?)
                .bind(person)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
}
