use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::core::UnixMillis;
use crate::core::client::filter::FilterDefinition;
use crate::core::identifiers::*;
use crate::core::serde::JsonValue;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_filters)]
pub struct DbUserFilter {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub filter: JsonValue,
    pub created_at: UnixMillis,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_filters)]
pub struct NewDbUserFilter {
    pub user_id: OwnedUserId,
    pub filter: JsonValue,
    pub created_at: UnixMillis,
}

pub async fn get_filter(user_id: &UserId, filter_id: i64) -> DataResult<FilterDefinition> {
    let filter = user_filters::table
        .filter(user_filters::id.eq(filter_id))
        .filter(user_filters::user_id.eq(user_id))
        .select(user_filters::filter)
        .first(&mut connect().await?)
        .await?;
    Ok(serde_json::from_value(filter)?)
}

pub async fn create_filter(user_id: &UserId, filter: &FilterDefinition) -> DataResult<i64> {
    let filter = diesel::insert_into(user_filters::table)
        .values(NewDbUserFilter {
            user_id: user_id.to_owned(),
            filter: serde_json::to_value(filter)?,
            created_at: UnixMillis::now(),
        })
        .get_result::<DbUserFilter>(&mut connect().await?)
        .await?;
    Ok(filter.id)
}
