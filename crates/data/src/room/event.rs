use horme::RunQueryDsl;

use crate::DataResult;
use crate::room::NewDbEventPushAction;
use crate::schema::*;

#[tracing::instrument]
pub async fn upsert_push_action(action: &NewDbEventPushAction) -> DataResult<()> {
    diesel::insert_into(event_push_actions::table)
        .values(action)
        .on_conflict_do_nothing()
        .execute(&mut crate::db())
        .await?;
    Ok(())
}
