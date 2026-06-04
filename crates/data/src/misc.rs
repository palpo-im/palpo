use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::serde::JsonValue;
use crate::core::{OwnedServerName, ServerName, UnixMillis};
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = server_signing_keys, primary_key(server_id))]
pub struct DbServerSigningKeys {
    pub server_id: OwnedServerName,
    pub key_data: JsonValue,
    pub updated_at: UnixMillis,
    pub created_at: UnixMillis,
}

/// Fetch the raw signing-key JSON blob stored for a server, if any.
pub async fn signing_keys_data(server: &ServerName) -> DataResult<Option<JsonValue>> {
    server_signing_keys::table
        .filter(server_signing_keys::server_id.eq(server))
        .select(server_signing_keys::key_data)
        .first::<JsonValue>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Insert or update the signing-key JSON blob for a server.
///
/// Callers are responsible for any merge logic; this performs a plain upsert of
/// the already-merged `key_data`.
pub async fn upsert_signing_keys(server: &ServerName, key_data: JsonValue) -> DataResult<()> {
    diesel::insert_into(server_signing_keys::table)
        .values(DbServerSigningKeys {
            server_id: server.to_owned(),
            key_data: key_data.clone(),
            updated_at: UnixMillis::now(),
            created_at: UnixMillis::now(),
        })
        .on_conflict(server_signing_keys::server_id)
        .do_update()
        .set((
            server_signing_keys::key_data.eq(key_data),
            server_signing_keys::updated_at.eq(UnixMillis::now()),
        ))
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}
