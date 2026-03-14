-- OAuth2 Authorization Server tables for MSC3861 Element X OIDC support

-- Dynamically registered OAuth2 clients (RFC 7591)
CREATE TABLE oauth_clients (
    client_id VARCHAR(255) PRIMARY KEY,
    client_name VARCHAR(255),
    redirect_uris TEXT NOT NULL,
    token_endpoint_auth_method VARCHAR(50) NOT NULL DEFAULT 'none',
    grant_types TEXT NOT NULL DEFAULT '["authorization_code","refresh_token"]',
    response_types TEXT NOT NULL DEFAULT '["code"]',
    application_type VARCHAR(50) DEFAULT 'native',
    last_used_at BIGINT,
    created_at BIGINT NOT NULL
);

-- Temporary authorization codes issued by Palpo AS
CREATE TABLE oauth_authorization_codes (
    code VARCHAR(255) PRIMARY KEY,
    client_id VARCHAR(255) NOT NULL REFERENCES oauth_clients(client_id),
    user_id VARCHAR(255) NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge VARCHAR(255) NOT NULL,
    code_challenge_method VARCHAR(10) NOT NULL DEFAULT 'S256',
    scope VARCHAR(512) NOT NULL DEFAULT 'urn:matrix:org.matrix.msc2967.client:api:*',
    expires_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL
);

-- Link access tokens to OAuth2 clients
ALTER TABLE user_access_tokens ADD COLUMN oauth_client_id VARCHAR(255) DEFAULT NULL;
