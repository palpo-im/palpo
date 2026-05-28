use diesel_async::RunQueryDsl;

use crate::room::NewDbEventPushAction;
use crate::schema::*;
use crate::{DataResult, connect};

#[tracing::instrument]
pub async fn upsert_push_action(action: &NewDbEventPushAction) -> DataResult<()> {
    diesel::insert_into(event_push_actions::table)
        .values(action)
        .on_conflict_do_nothing()
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}
