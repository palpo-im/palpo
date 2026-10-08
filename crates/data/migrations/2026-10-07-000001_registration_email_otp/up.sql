CREATE TABLE registration_email_sessions (
    sid text PRIMARY KEY,
    email text NOT NULL,
    client_secret_hash text NOT NULL,
    code_hash text NOT NULL,
    send_attempt bigint NOT NULL,
    created_at bigint NOT NULL,
    expires_at bigint NOT NULL,
    sent_at bigint,
    failed_attempts integer NOT NULL DEFAULT 0,
    verified_at bigint,
    claimed_session text UNIQUE,
    claimed_user_id text,
    consumed_at bigint
);
CREATE UNIQUE INDEX registration_email_attempt ON registration_email_sessions (email, client_secret_hash, send_attempt);
CREATE INDEX registration_email_recipient_time
    ON registration_email_sessions (email, created_at DESC);
CREATE INDEX registration_email_expiry ON registration_email_sessions (expires_at);
