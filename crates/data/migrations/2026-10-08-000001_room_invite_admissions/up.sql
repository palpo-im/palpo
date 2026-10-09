-- An invitation that qualified for sync stays admitted until that membership is replaced.
-- Scope the decision to the membership row, so a new invite cannot inherit old trust.
CREATE TABLE room_invite_admissions (
    room_user_id BIGINT PRIMARY KEY REFERENCES room_users(id) ON DELETE CASCADE,
    admitted_sn BIGINT NOT NULL
);
