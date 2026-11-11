//! Publication of reserved uploads. Storage is written before publishing its
//! metadata, under the same cross-instance fence used by media deletion.

use crate::core::{ServerName, UnixMillis, UserId};
use crate::data::media::{NewDbMetadata, UploadReservation};
use crate::{AppError, AppResult, MatrixError, data, storage};

fn authorize(
    reservation: Option<&UploadReservation>,
    owner: &UserId,
    started_at: UnixMillis,
) -> AppResult<()> {
    let reservation =
        reservation.ok_or_else(|| MatrixError::not_found("Media reservation not found"))?;
    if reservation.owner_id != owner {
        return Err(MatrixError::forbidden("Media belongs to another user", None).into());
    }
    if reservation.uploaded {
        return Err(MatrixError::cannot_overwrite_media("Media ID already has content").into());
    }
    if reservation.expires_at < started_at.0 as i64 {
        return Err(MatrixError::not_found("Media reservation expired").into());
    }
    Ok(())
}

pub(crate) async fn check_upload_reservation(
    server: &ServerName,
    media_id: &str,
    owner: &UserId,
    started_at: UnixMillis,
) -> AppResult<()> {
    let mut conn = data::connect();
    let reservation = data::media::upload_reservation(&mut conn, server, media_id).await?;
    authorize(reservation.as_ref(), owner, started_at)
}

pub(crate) async fn publish_reserved_upload(
    metadata: NewDbMetadata,
    payload: Vec<u8>,
    started_at: UnixMillis,
) -> AppResult<()> {
    let operator = storage::operator().clone();
    let storage_origin = super::effective_server_name(&metadata.origin_server).to_owned();
    // Once storage publication starts, finish even if the HTTP request is
    // cancelled. Otherwise an in-flight object write could outlive its DB lock.
    tokio::spawn(async move {
        publish_with_operator(&operator, &storage_origin, metadata, &payload, started_at).await
    })
    .await
    .map_err(|error| AppError::internal(format!("media upload task failed: {error}")))?
}

async fn publish_with_operator(
    operator: &opendal::Operator,
    storage_origin: &str,
    metadata: NewDbMetadata,
    payload: &[u8],
    started_at: UnixMillis,
) -> AppResult<()> {
    let server = metadata.origin_server.clone();
    let media_id = metadata.media_id.clone();
    super::coordination::with_lock(&server, &media_id, async {
        // Never replace an already published object, including legacy metadata.
        if data::media::get_metadata(&server, &media_id)
            .await?
            .is_some()
        {
            return Err(MatrixError::cannot_overwrite_media("Media ID already has content").into());
        }
        data::connect()
            .transaction::<_, AppError, _>(async |conn| {
                let reservation =
                    data::media::lock_upload_reservation(conn, &server, &media_id).await?;
                let owner = metadata
                    .created_by
                    .as_deref()
                    .ok_or_else(|| AppError::internal("upload owner missing"))?;
                authorize(reservation.as_ref(), owner, started_at)?;
                let key = storage::media_key(storage_origin, &media_id);
                // A failed/crashed previous publication can leave an unreferenced
                // object here. It is not downloadable without metadata and only the
                // same reserved owner can retry it under this fence.
                operator.write(&key, payload.to_vec()).await?;
                data::media::complete_upload(conn, &metadata).await?;
                Ok(())
            })
            .await
    })
    .await
}

#[cfg(test)]
mod tests {
    use horme::RunQueryDsl;

    use super::*;

    #[tokio::test]
    #[ignore = "requires an empty dedicated PALPO_TEST_DATABASE_URL; run alone"]
    async fn review_reserved_upload_serializes_and_recovers_storage_and_db_failures() {
        crate::test_database::init();
        let dir = tempfile::tempdir().unwrap();
        let operator = opendal::Operator::new(
            opendal::services::Fs::default().root(dir.path().to_str().unwrap()),
        )
        .unwrap();
        let server: crate::core::OwnedServerName = "media.example".try_into().unwrap();
        let owner: crate::core::OwnedUserId = "@owner:media.example".try_into().unwrap();
        let now = UnixMillis::now();
        let metadata = NewDbMetadata {
            media_id: "reserved".into(),
            origin_server: server.clone(),
            content_type: Some("text/plain".into()),
            disposition_type: Some("inline".into()),
            file_name: None,
            file_extension: None,
            file_size: 3,
            file_hash: None,
            created_by: Some(owner.clone()),
            created_at: now,
        };
        data::media::reserve_upload(&server, "reserved", &owner, UnixMillis(now.0 + 60_000))
            .await
            .unwrap();
        let bad = opendal::Operator::new(
            opendal::services::Fs::default().root(dir.path().join("blocked").to_str().unwrap()),
        )
        .unwrap();
        bad.write("media", b"file blocks storage directory".to_vec())
            .await
            .unwrap();
        assert!(
            publish_with_operator(&bad, "_", metadata.clone(), b"one", now)
                .await
                .is_err()
        );
        assert!(
            data::media::get_metadata(&server, "reserved")
                .await
                .unwrap()
                .is_none()
        );
        check_upload_reservation(&server, "reserved", &owner, now)
            .await
            .unwrap();

        // A DB failure after the object write must not publish a successful upload.
        diesel::sql_query("CREATE FUNCTION reject_media_upload() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected metadata failure'; END $$")
            .execute(&mut crate::data::db()).await.unwrap();
        diesel::sql_query("CREATE TRIGGER reject_media_upload BEFORE INSERT ON media_metadatas FOR EACH ROW EXECUTE FUNCTION reject_media_upload()")
            .execute(&mut crate::data::db()).await.unwrap();
        assert!(
            publish_with_operator(&operator, "_", metadata.clone(), b"old", now)
                .await
                .is_err()
        );
        assert!(
            data::media::get_metadata(&server, "reserved")
                .await
                .unwrap()
                .is_none()
        );
        check_upload_reservation(&server, "reserved", &owner, now)
            .await
            .unwrap();
        diesel::sql_query("DROP TRIGGER reject_media_upload ON media_metadatas")
            .execute(&mut crate::data::db())
            .await
            .unwrap();

        let (first, second) = tokio::join!(
            publish_with_operator(&operator, "_", metadata.clone(), b"one", now),
            publish_with_operator(&operator, "_", metadata, b"two", now),
        );
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let bytes = operator
            .read(&storage::media_key("_", "reserved"))
            .await
            .unwrap()
            .to_vec();
        assert_eq!(bytes, if first.is_ok() { b"one" } else { b"two" });
        let saved = data::media::get_metadata(&server, "reserved")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.created_by, Some(owner.clone()));
        assert_eq!(saved.file_size, bytes.len() as i64);
        assert!(
            check_upload_reservation(&server, "reserved", &owner, now)
                .await
                .is_err()
        );
    }
}
