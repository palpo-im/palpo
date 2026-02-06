use diesel::prelude::*;

use crate::core::UnixMillis;
use crate::core::identifiers::*;
use crate::schema::*;
use crate::{DataResult, connect};

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = media_metadatas)]
pub struct DbMetadata {
    pub id: i64,
    pub media_id: String,
    pub origin_server: OwnedServerName,
    pub content_type: Option<String>,
    pub disposition_type: Option<String>,
    pub file_name: Option<String>,
    pub file_extension: Option<String>,
    pub file_size: i64,
    pub file_hash: Option<String>,
    pub created_by: Option<OwnedUserId>,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = media_metadatas)]
pub struct NewDbMetadata {
    pub media_id: String,
    pub origin_server: OwnedServerName,
    pub content_type: Option<String>,
    pub disposition_type: Option<String>,
    pub file_name: Option<String>,
    pub file_extension: Option<String>,
    pub file_size: i64,
    pub file_hash: Option<String>,
    pub created_by: Option<OwnedUserId>,
    pub created_at: UnixMillis,
}

pub fn get_metadata(server_name: &ServerName, media_id: &str) -> DataResult<Option<DbMetadata>> {
    media_metadatas::table
        .filter(media_metadatas::media_id.eq(media_id))
        .filter(media_metadatas::origin_server.eq(server_name))
        .first::<DbMetadata>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

pub fn delete_media(server_name: &ServerName, media_id: &str) -> DataResult<()> {
    diesel::delete(
        media_metadatas::table
            .filter(media_metadatas::media_id.eq(media_id))
            .filter(media_metadatas::origin_server.eq(server_name)),
    )
    .execute(&mut connect()?)?;
    Ok(())
}

#[derive(Insertable, Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = media_thumbnails)]
pub struct DbThumbnail {
    pub id: i64,
    pub media_id: String,
    pub origin_server: OwnedServerName,
    pub content_type: Option<String>,
    pub disposition_type: Option<String>,
    pub file_size: i64,
    pub width: i32,
    pub height: i32,
    pub resize_method: String,
    pub created_at: UnixMillis,
}
#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = media_thumbnails)]
pub struct NewDbThumbnail {
    pub media_id: String,
    pub origin_server: OwnedServerName,
    pub content_type: Option<String>,
    pub disposition_type: Option<String>,
    pub file_size: i64,
    pub width: i32,
    pub height: i32,
    pub resize_method: String,
    pub created_at: UnixMillis,
}

pub fn get_thumbnail_by_dimension(
    origin_server: &ServerName,
    media_id: &str,
    width: u32,
    height: u32,
) -> DataResult<Option<DbThumbnail>> {
    media_thumbnails::table
        .filter(media_thumbnails::origin_server.eq(origin_server))
        .filter(media_thumbnails::media_id.eq(media_id))
        .filter(media_thumbnails::width.eq(width as i32))
        .filter(media_thumbnails::height.eq(height as i32))
        .first::<DbThumbnail>(&mut connect()?)
        .optional()
        .map_err(Into::into)
}

#[derive(Identifiable, Queryable, Debug, Clone)]
#[diesel(table_name = media_url_previews)]
pub struct DbUrlPreview {
    pub id: i64,
    pub url: String,
    pub og_title: Option<String>,
    pub og_type: Option<String>,
    pub og_url: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub image_size: Option<i64>,
    pub og_image_width: Option<i32>,
    pub og_image_height: Option<i32>,
    pub created_at: UnixMillis,
}

#[derive(Insertable, AsChangeset, Debug, Clone)]
#[diesel(table_name = media_url_previews)]
pub struct NewDbUrlPreview {
    pub url: String,
    pub og_title: Option<String>,
    pub og_type: Option<String>,
    pub og_url: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub image_size: Option<i64>,
    pub og_image_width: Option<i32>,
    pub og_image_height: Option<i32>,
    pub created_at: UnixMillis,
}

pub fn get_url_preview(url: &str) -> DataResult<DbUrlPreview> {
    media_url_previews::table
        .filter(media_url_previews::url.eq(url))
        .first::<DbUrlPreview>(&mut connect()?)
        .map_err(Into::into)
}

pub fn set_url_preview(preview: &NewDbUrlPreview) -> DataResult<()> {
    diesel::insert_into(media_url_previews::table)
        .values(preview)
        .on_conflict(media_url_previews::url)
        .do_update()
        .set(preview)
        .execute(&mut connect()?)?;
    Ok(())
}

pub fn insert_metadata(metadata: &NewDbMetadata) -> DataResult<()> {
    diesel::insert_into(media_metadatas::table)
        .values(metadata)
        .execute(&mut connect()?)?;
    Ok(())
}

/// List media uploaded by a user with pagination
pub fn list_media_by_user(
    user_id: &UserId,
    from: i64,
    limit: i64,
    order_by: Option<&str>,
    direction: Option<&str>,
) -> DataResult<(Vec<DbMetadata>, i64)> {
    // Get total count
    let total = media_metadatas::table
        .filter(media_metadatas::created_by.eq(user_id.as_str()))
        .count()
        .get_result::<i64>(&mut connect()?)?;

    // Build query with ordering
    let direction_desc = direction.map(|d| d == "b").unwrap_or(true);
    let mut query = media_metadatas::table
        .filter(media_metadatas::created_by.eq(user_id.as_str()))
        .into_boxed();

    query = match order_by {
        Some("media_id") => {
            if direction_desc {
                query.order(media_metadatas::media_id.desc())
            } else {
                query.order(media_metadatas::media_id.asc())
            }
        }
        Some("upload_name") => {
            if direction_desc {
                query.order(media_metadatas::file_name.desc())
            } else {
                query.order(media_metadatas::file_name.asc())
            }
        }
        Some("media_length") => {
            if direction_desc {
                query.order(media_metadatas::file_size.desc())
            } else {
                query.order(media_metadatas::file_size.asc())
            }
        }
        Some("media_type") => {
            if direction_desc {
                query.order(media_metadatas::content_type.desc())
            } else {
                query.order(media_metadatas::content_type.asc())
            }
        }
        _ => {
            // Default: created_ts
            if direction_desc {
                query.order(media_metadatas::created_at.desc())
            } else {
                query.order(media_metadatas::created_at.asc())
            }
        }
    };

    let media = query
        .offset(from)
        .limit(limit)
        .load::<DbMetadata>(&mut connect()?)?;

    Ok((media, total))
}

/// Delete multiple media items by their IDs
/// Returns the list of deleted media IDs and total count
pub fn delete_media_by_ids(
    server_name: &ServerName,
    media_ids: &[String],
) -> DataResult<(Vec<String>, i64)> {
    let mut deleted = Vec::new();

    for media_id in media_ids {
        let rows = diesel::delete(
            media_metadatas::table
                .filter(media_metadatas::media_id.eq(media_id))
                .filter(media_metadatas::origin_server.eq(server_name)),
        )
        .execute(&mut connect()?)?;

        if rows > 0 {
            // Also delete thumbnails
            diesel::delete(
                media_thumbnails::table
                    .filter(media_thumbnails::media_id.eq(media_id))
                    .filter(media_thumbnails::origin_server.eq(server_name)),
            )
            .execute(&mut connect()?)?;
            deleted.push(media_id.clone());
        }
    }

    let total = deleted.len() as i64;
    Ok((deleted, total))
}

/// Purge old remote media cache (media from other servers)
/// Returns the count of deleted items
pub fn purge_remote_media_cache(local_server: &ServerName, before_ts: i64) -> DataResult<i64> {
    let before_ts = UnixMillis(before_ts as u64);

    // Delete thumbnails first
    let deleted_thumbnails = diesel::delete(
        media_thumbnails::table
            .filter(media_thumbnails::origin_server.ne(local_server))
            .filter(media_thumbnails::created_at.lt(before_ts)),
    )
    .execute(&mut connect()?)? as i64;

    // Delete metadata
    let deleted_metadata = diesel::delete(
        media_metadatas::table
            .filter(media_metadatas::origin_server.ne(local_server))
            .filter(media_metadatas::created_at.lt(before_ts)),
    )
    .execute(&mut connect()?)? as i64;

    Ok(deleted_metadata + deleted_thumbnails)
}

/// Delete old local media before a timestamp and larger than size_gt
pub fn delete_old_local_media(
    local_server: &ServerName,
    before_ts: i64,
    size_gt: i64,
) -> DataResult<(Vec<String>, i64)> {
    let before_ts = UnixMillis(before_ts as u64);

    // Get media IDs to delete
    let media_ids = media_metadatas::table
        .filter(media_metadatas::origin_server.eq(local_server))
        .filter(media_metadatas::created_at.lt(before_ts))
        .filter(media_metadatas::file_size.gt(size_gt))
        .select(media_metadatas::media_id)
        .load::<String>(&mut connect()?)?;

    if media_ids.is_empty() {
        return Ok((vec![], 0));
    }

    // Delete thumbnails
    diesel::delete(
        media_thumbnails::table
            .filter(media_thumbnails::origin_server.eq(local_server))
            .filter(media_thumbnails::media_id.eq_any(&media_ids)),
    )
    .execute(&mut connect()?)?;

    // Delete metadata
    diesel::delete(
        media_metadatas::table
            .filter(media_metadatas::origin_server.eq(local_server))
            .filter(media_metadatas::media_id.eq_any(&media_ids)),
    )
    .execute(&mut connect()?)?;

    let total = media_ids.len() as i64;
    Ok((media_ids, total))
}

#[derive(diesel::QueryableByName)]
pub struct UserMediaStatsRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub user_id: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub media_count: i64,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub media_length: i64,
}

pub fn get_user_media_statistics(
    offset: i64,
    limit: i64,
    search_term: Option<&str>,
    order_by: Option<&str>,
    dir: Option<&str>,
) -> DataResult<(Vec<UserMediaStatsRow>, i64)> {
    let mut conn = connect()?;
    let order_col = match order_by {
        Some("media_count") => "media_count",
        _ => "media_length",
    };
    let order_dir = match dir {
        Some("f") => "ASC",
        _ => "DESC",
    };
    let search_filter = search_term
        .map(|s| format!("AND created_by LIKE '%{}%'", s.replace('\'', "''")))
        .unwrap_or_default();

    let count_sql = format!(
        "SELECT COUNT(*) FROM (\
            SELECT created_by FROM media_metadatas \
            WHERE created_by IS NOT NULL {} \
            GROUP BY created_by\
        ) sub",
        search_filter
    );
    let total: i64 = diesel::sql_query(&count_sql)
        .get_result::<CountResult>(&mut conn)?
        .count;

    let data_sql = format!(
        "SELECT created_by AS user_id, \
            COUNT(*) AS media_count, \
            COALESCE(SUM(file_size), 0) AS media_length \
        FROM media_metadatas \
        WHERE created_by IS NOT NULL {} \
        GROUP BY created_by \
        ORDER BY {} {} \
        LIMIT {} OFFSET {}",
        search_filter, order_col, order_dir, limit, offset
    );
    let rows = diesel::sql_query(&data_sql)
        .load::<UserMediaStatsRow>(&mut conn)?;
    Ok((rows, total))
}

#[derive(diesel::QueryableByName)]
struct CountResult {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}
