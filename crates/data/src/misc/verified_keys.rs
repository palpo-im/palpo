use diesel::sql_types::{BigInt, Integer, Jsonb, Text};
use diesel_async::RunQueryDsl;

use super::*;

#[derive(diesel::QueryableByName)]
pub struct VerifiedKeyDocument {
    #[diesel(sql_type = Jsonb)]
    pub key_data: JsonValue,
    #[diesel(sql_type = Text)]
    pub document_sha256: String,
    #[diesel(sql_type = Text)]
    pub source: String,
    #[diesel(sql_type = Text)]
    pub source_server: String,
    #[diesel(sql_type = Integer)]
    pub validation_version: i32,
    #[diesel(sql_type = BigInt)]
    pub validated_at: i64,
}

pub async fn verified_key_documents(server: &ServerName) -> DataResult<Vec<VerifiedKeyDocument>> {
    Ok(diesel::sql_query("SELECT key_data,document_sha256,source,source_server,validation_version,validated_at FROM verified_server_key_documents WHERE server_id=$1 ORDER BY validated_at DESC,id DESC")
        .bind::<Text,_>(server.as_str()).load(&mut connect().await?).await?)
}

/// Preserve signed bodies separately from the legacy merged key index.
pub async fn store_verified_key_document(
    server: &ServerName,
    raw: &JsonValue,
    digest: &str,
    source: &str,
    source_server: &ServerName,
) -> DataResult<()> {
    diesel::sql_query("INSERT INTO verified_server_key_documents(server_id,document_sha256,key_data,source,source_server,validation_version,validated_at) VALUES($1,$2,$3,$4,$5,1,$6) ON CONFLICT(server_id,document_sha256) DO UPDATE SET validated_at=EXCLUDED.validated_at,source=EXCLUDED.source,source_server=EXCLUDED.source_server,validation_version=1")
        .bind::<Text,_>(server.as_str()).bind::<Text,_>(digest).bind::<Jsonb,_>(raw)
        .bind::<Text,_>(source).bind::<Text,_>(source_server.as_str()).bind::<BigInt,_>(UnixMillis::now().0.min(i64::MAX as u64) as i64)
        .execute(&mut connect().await?).await?;
    Ok(())
}
