use diesel::prelude::*;

use crate::schema::*;

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = oauth_clients, primary_key(client_id))]
pub struct DbOAuthClient {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: String,
    pub token_endpoint_auth_method: String,
    pub grant_types: String,
    pub response_types: String,
    pub application_type: Option<String>,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = oauth_clients)]
pub struct NewDbOAuthClient {
    pub client_id: String,
    pub client_name: Option<String>,
    pub redirect_uris: String,
    pub token_endpoint_auth_method: String,
    pub grant_types: String,
    pub response_types: String,
    pub application_type: Option<String>,
    pub created_at: i64,
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = oauth_authorization_codes, primary_key(code))]
pub struct DbOAuthAuthorizationCode {
    pub code: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = oauth_authorization_codes)]
pub struct NewDbOAuthAuthorizationCode {
    pub code: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: String,
    pub expires_at: i64,
    pub created_at: i64,
}
