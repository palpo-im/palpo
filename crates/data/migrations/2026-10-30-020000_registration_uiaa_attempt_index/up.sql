-- Registration fingerprints contain no credentials and are always 64 ASCII
-- bytes. Keep other UIAA request bodies and their arbitrary fields out of this
-- index; the lookup/prune queries use the same bounded-key predicate.
CREATE INDEX user_uiaa_registration_attempt_idx
    ON user_uiaa_datas (
        user_id,
        device_id,
        (request_body->'palpo.registration'->>'fingerprint'),
        id DESC
    )
    WHERE octet_length(request_body->'palpo.registration'->>'fingerprint') = 64;
