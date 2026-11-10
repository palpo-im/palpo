use diesel::sql_types::{Array, BigInt, Bool, Integer, Jsonb, Nullable, Text};
use diesel_async::{AsyncConnection, RunQueryDsl};

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

const FIELDS: &str =
    "d.key_data,d.document_sha256,d.source,d.source_server,d.validation_version,d.validated_at";
const TRUST: &str = "d.validation_version=1 AND d.validated_at>=0 AND ((d.source='direct' AND d.source_server=d.server_id) OR (d.source='notary' AND d.source_server=ANY($2)))";

/// Online HTTP/publication reads load exactly the latest document overall.
pub async fn latest_verified_key_document(
    server: &ServerName,
    notaries: &[String],
) -> DataResult<Vec<VerifiedKeyDocument>> {
    Ok(diesel::sql_query(format!("SELECT {FIELDS} FROM verified_server_key_documents d WHERE d.server_id=$1 AND {TRUST} ORDER BY d.validated_at DESC,d.id DESC LIMIT 1"))
        .bind::<Text,_>(server.as_str()).bind::<Array<Text>,_>(notaries).load(&mut connect().await?).await?)
}

pub async fn verified_key_retirement(
    server: &ServerName,
    id: &str,
    notaries: &[String],
) -> DataResult<Option<i64>> {
    #[derive(diesel::QueryableByName)]
    struct Retirement {
        #[diesel(sql_type=Nullable<BigInt>)]
        expiry: Option<i64>,
    }
    Ok(diesel::sql_query(format!("SELECT min(LEAST(v.valid_until::numeric,d.validated_at::numeric+604800000))::bigint AS expiry FROM verified_server_key_versions v JOIN verified_server_key_documents d ON d.id=v.document_id AND d.server_id=v.server_id WHERE v.server_id=$1 AND v.key_id=$3 AND v.retired AND {TRUST}"))
        .bind::<Text,_>(server.as_str()).bind::<Array<Text>,_>(notaries).bind::<Text,_>(id).get_result::<Retirement>(&mut connect().await?).await?.expiry)
}

/// A fixed online budget. Historical rows remain available to offline tooling.
pub const EVENT_KEY_DOCUMENT_BUDGET: i64 = 8;

pub async fn verified_event_key_documents(
    server: &ServerName,
    id: &str,
    timestamp: i64,
    enforce: bool,
    notaries: &[String],
) -> DataResult<Vec<VerifiedKeyDocument>> {
    Ok(diesel::sql_query(format!("SELECT {FIELDS} FROM verified_server_key_versions v JOIN verified_server_key_documents d ON d.id=v.document_id AND d.server_id=v.server_id WHERE v.server_id=$1 AND v.key_id=$3 AND {TRUST} AND (NOT $5 OR ((CASE WHEN v.retired THEN v.valid_until>$4 ELSE v.valid_until>=$4 END) AND d.validated_at::numeric+604800000 >= $4::numeric)) ORDER BY d.validated_at DESC,d.id DESC LIMIT 8"))
        .bind::<Text,_>(server.as_str()).bind::<Array<Text>,_>(notaries).bind::<Text,_>(id).bind::<BigInt,_>(timestamp).bind::<Bool,_>(enforce)
        .load(&mut connect().await?).await?)
}

/// Preserve signed bodies separately from the legacy merged key index.
pub async fn store_verified_key_document(
    server: &ServerName,
    raw: &JsonValue,
    digest: &str,
    source: &str,
    source_server: &ServerName,
) -> DataResult<()> {
    #[derive(diesel::QueryableByName)]
    struct DocumentId {
        #[diesel(sql_type=BigInt)]
        id: i64,
    }
    connect().await?.transaction::<_,crate::DataError,_>(async |conn| {
    let document=diesel::sql_query("INSERT INTO verified_server_key_documents(server_id,document_sha256,key_data,source,source_server,validation_version,validated_at) VALUES($1,$2,$3,$4,$5,1,$6) ON CONFLICT(server_id,document_sha256) DO UPDATE SET validated_at=EXCLUDED.validated_at,source=EXCLUDED.source,source_server=EXCLUDED.source_server,validation_version=1 RETURNING id")
        .bind::<Text,_>(server.as_str()).bind::<Text,_>(digest).bind::<Jsonb,_>(raw)
        .bind::<Text,_>(source).bind::<Text,_>(source_server.as_str()).bind::<BigInt,_>(UnixMillis::now().0.min(i64::MAX as u64) as i64)
        .get_result::<DocumentId>(conn).await?;
    // Populate or repair the immutable document's selector index atomically,
    // including concurrent duplicate observations. No trust/source is copied
    // into this index: readers always join the document's current metadata.
    diesel::sql_query("INSERT INTO verified_server_key_versions(document_id,server_id,key_id,retired,valid_until) SELECT $1,$2,key,false,LEAST(($3::jsonb->>'valid_until_ts')::numeric,9223372036854775807)::bigint FROM jsonb_each($3::jsonb->'verify_keys') UNION ALL SELECT $1,$2,key,true,LEAST((value->>'expired_ts')::numeric,9223372036854775807)::bigint FROM jsonb_each($3::jsonb->'old_verify_keys') WHERE true ON CONFLICT(document_id,key_id,retired) DO NOTHING")
        .bind::<BigInt,_>(document.id).bind::<Text,_>(server.as_str()).bind::<Jsonb,_>(raw).execute(conn).await?;
    Ok(()) }).await
}
