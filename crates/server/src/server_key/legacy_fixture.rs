//! Test-only access to retained, unproven pre-migration cache rows.
//!
//! No production read/write helper exists: key retrieval must preserve and
//! validate raw documents through the verified document store.
use diesel::sql_types::{Json, Text};
use horme::{OptionalExtension as _, RunQueryDsl};
use serde_json::Value;

use crate::core::ServerName;
use crate::{AppResult, data};

pub(super) async fn document(server: &ServerName) -> AppResult<Option<Value>> {
    #[derive(diesel::QueryableByName)]
    struct Legacy {
        #[diesel(sql_type = Json)]
        key_data: Value,
    }
    Ok(
        diesel::sql_query("SELECT key_data FROM server_signing_keys WHERE server_id=$1")
            .bind::<Text, _>(server.as_str())
            .get_result::<Legacy>(&mut data::connect())
            .await
            .optional()?
            .map(|record| record.key_data),
    )
}

pub(super) async fn store(server: &ServerName, value: Value) -> AppResult<()> {
    diesel::sql_query("INSERT INTO server_signing_keys(server_id,key_data,updated_at,created_at) VALUES($1,$2,0,0)")
        .bind::<Text, _>(server.as_str())
        .bind::<Json, _>(value)
        .execute(&mut data::connect())
        .await?;
    Ok(())
}

pub(super) async fn reject_writes() -> AppResult<()> {
    use diesel_async::SimpleAsyncConnection;

    data::checkout().await?.batch_execute("CREATE FUNCTION reject_legacy_key_writes() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'legacy key cache is read-only'; END $$;
        CREATE TRIGGER reject_legacy_key_writes BEFORE INSERT OR UPDATE OR DELETE ON server_signing_keys FOR EACH ROW EXECUTE FUNCTION reject_legacy_key_writes()").await?;
    Ok(())
}
