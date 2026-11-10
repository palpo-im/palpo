-- Never promote the legacy merged cache: its source and signatures are unknown.
CREATE TABLE verified_server_key_documents (
    id BIGSERIAL PRIMARY KEY,
    server_id TEXT NOT NULL,
    document_sha256 TEXT NOT NULL,
    key_data JSONB NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('direct', 'notary')),
    source_server TEXT NOT NULL,
    validation_version INTEGER NOT NULL,
    validated_at BIGINT NOT NULL,
    UNIQUE (server_id, document_sha256)
);
CREATE INDEX verified_server_key_documents_lookup
    ON verified_server_key_documents(server_id, validated_at DESC, id DESC);
