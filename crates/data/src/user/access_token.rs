use diesel::prelude::*;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = user_access_tokens)]
pub struct DbAccessToken {
    pub id: i64,
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub token: String,
    pub puppets_user_id: Option<OwnedUserId>,
    pub last_validated: Option<UnixMillis>,
    pub refresh_token_id: Option<i64>,
    pub is_used: bool,
    pub expires_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
    pub oauth_client_id: Option<String>,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = user_access_tokens)]
pub struct NewDbAccessToken {
    pub user_id: OwnedUserId,
    pub device_id: OwnedDeviceId,
    pub token: String,
    pub puppets_user_id: Option<OwnedUserId>,
    pub last_validated: Option<UnixMillis>,
    pub refresh_token_id: Option<i64>,
    pub is_used: bool,
    pub expires_at: Option<UnixMillis>,
    pub created_at: UnixMillis,
    pub oauth_client_id: Option<String>,
}

impl NewDbAccessToken {
    pub fn new(
        user_id: OwnedUserId,
        device_id: OwnedDeviceId,
        token: String,
        refresh_token_id: Option<i64>,
    ) -> Self {
        Self {
            user_id,
            device_id,
            token,
            puppets_user_id: None,
            last_validated: None,
            refresh_token_id,
            is_used: false,
            expires_at: None,
            created_at: UnixMillis::now(),
            oauth_client_id: None,
        }
    }
}
