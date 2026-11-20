//! WP-34 assisted channels (FR-CH-03, FR-CH-04): branch, private banker and call center staff act for a customer only
//! inside a session the customer authorized in a bank channel. The session actor keeps the staff identity and names the
//! represented customer and channel; Controls re-checks session, channel policy and account restrictions per decision.
use anyhow::Result;
use bank_platform::domain::{ActorContext, PartyId};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Assisted {
    pub pool: PgPool,
}
impl Assisted {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// The staff actor acting inside a session: same staff identity, the customer as represented party, the channel
    /// as client, and `assisted:<session>` for the decision-time check.
    pub fn actor(
        staff: &ActorContext,
        session: Uuid,
        channel: &str,
        customer: PartyId,
    ) -> ActorContext {
        let mut actor = staff.clone();
        actor.client_id = format!(
            "assisted-{}",
            channel.to_ascii_lowercase().replace('_', "-")
        );
        actor.represented_party = Some(customer);
        actor.roles = vec![format!("assisted:{session}")];
        actor
    }
    pub async fn open(
        &self,
        staff: &ActorContext,
        person: PartyId,
        key: Uuid,
        data: &Value,
    ) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT controls.open_assisted_session($1,$2,$3,$4)")
                .bind(serde_json::to_value(staff)?)
                .bind(person.0)
                .bind(key)
                .bind(data)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn authorize(
        &self,
        customer: &ActorContext,
        person: PartyId,
        id: Uuid,
        approve: bool,
    ) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT controls.authorize_assisted_session($1,$2,$3,$4)")
                .bind(serde_json::to_value(customer)?)
                .bind(person.0)
                .bind(id)
                .bind(approve)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn close(&self, actor: &ActorContext, person: PartyId, id: Uuid) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT controls.close_assisted_session($1,$2,$3)")
                .bind(serde_json::to_value(actor)?)
                .bind(person.0)
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn staff_view(&self, staff: &ActorContext, person: PartyId) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT controls.assisted_staff_view($1,$2)")
                .bind(serde_json::to_value(staff)?)
                .bind(person.0)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn customer_view(&self, person: PartyId) -> Result<Value> {
        Ok(
            sqlx::query_scalar("SELECT controls.assisted_customer_view($1)")
                .bind(person.0)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    /// The session's channel and customer for a staff member's own session (any state), or `None`.
    pub async fn own_session(&self, staff: PartyId, id: Uuid) -> Result<Option<(String, PartyId)>> {
        let row: Option<(String, Uuid)> = sqlx::query_as(
            "SELECT channel,customer_person FROM controls.assisted_sessions WHERE id=$1 AND staff_person=$2",
        )
        .bind(id)
        .bind(staff.0)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(channel, customer)| (channel, PartyId(customer))))
    }
}
