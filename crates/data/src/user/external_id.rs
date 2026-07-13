use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_external_ids)]
pub struct DbUserExternalId {
    pub id: i64,
    pub auth_provider: String,
    pub external_id: String,
    pub user_id: OwnedUserId,
    pub created_at: UnixMillis,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_external_ids)]
pub struct NewDbUserExternalId {
    pub auth_provider: String,
    pub external_id: String,
    pub user_id: OwnedUserId,
    pub created_at: UnixMillis,
}

/// Get user_id by external auth provider and external_id
pub async fn get_user_by_external_id(
    auth_provider: &str,
    external_id: &str,
) -> DataResult<Option<OwnedUserId>> {
    user_external_ids::table
        .filter(user_external_ids::auth_provider.eq(auth_provider))
        .filter(user_external_ids::external_id.eq(external_id))
        .select(user_external_ids::user_id)
        .first::<OwnedUserId>(&mut connect().await?)
        .await
        .optional()
        .map_err(Into::into)
}

/// Get all external IDs for a user
pub async fn get_external_ids_by_user(user_id: &UserId) -> DataResult<Vec<DbUserExternalId>> {
    user_external_ids::table
        .filter(user_external_ids::user_id.eq(user_id))
        .load::<DbUserExternalId>(&mut connect().await?)
        .await
        .map_err(Into::into)
}

/// Record a new external ID for a user
pub async fn record_external_id(
    auth_provider: &str,
    external_id: &str,
    user_id: &UserId,
) -> DataResult<()> {
    diesel::insert_into(user_external_ids::table)
        .values(NewDbUserExternalId {
            auth_provider: auth_provider.to_owned(),
            external_id: external_id.to_owned(),
            user_id: user_id.to_owned(),
            created_at: UnixMillis::now(),
        })
        .execute(&mut connect().await?)
        .await?;
    Ok(())
}

/// Replace all external IDs for a user
pub async fn replace_external_ids(
    user_id: &UserId,
    new_external_ids: &[(String, String)], // (auth_provider, external_id)
) -> DataResult<()> {
    let mut conn = connect().await?;

    // Delete existing external IDs for this user
    diesel::delete(user_external_ids::table.filter(user_external_ids::user_id.eq(user_id)))
        .execute(&mut conn)
        .await?;

    // Insert new external IDs
    let now = UnixMillis::now();
    for (auth_provider, external_id) in new_external_ids {
        diesel::insert_into(user_external_ids::table)
            .values(NewDbUserExternalId {
                auth_provider: auth_provider.clone(),
                external_id: external_id.clone(),
                user_id: user_id.to_owned(),
                created_at: now,
            })
            .execute(&mut conn)
            .await?;
    }

    Ok(())
}

/// Delete a specific external ID
pub async fn delete_external_id(auth_provider: &str, external_id: &str) -> DataResult<()> {
    diesel::delete(
        user_external_ids::table
            .filter(user_external_ids::auth_provider.eq(auth_provider))
            .filter(user_external_ids::external_id.eq(external_id)),
    )
    .execute(&mut connect().await?)
    .await?;
    Ok(())
}
