//! Bounded federation selectors. Positions are committed with the outgoing ACK.

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Json, Nullable, Text};
use diesel_async::AsyncPgConnection;
use horme::RunQueryDsl;

use crate::DataResult;
use crate::core::identifiers::*;

/// `token = None` means the entire sequence number is confirmed. A token
/// resumes strictly after that user in a partially delivered device tie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorPosition {
    pub sn: i64,
    pub token: Option<String>,
}

impl SelectorPosition {
    pub fn complete(sn: i64) -> Self {
        Self { sn, token: None }
    }

    pub fn confirmed_through(&self) -> i64 {
        if self.token.is_some() {
            self.sn.saturating_sub(1)
        } else {
            self.sn
        }
    }
}

#[derive(QueryableByName)]
struct CursorRow {
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = BigInt)]
    sn: i64,
    #[diesel(sql_type = Nullable<Text>)]
    token: Option<String>,
}

pub async fn selector_positions(
    server: &ServerName,
    fallback: i64,
) -> DataResult<(SelectorPosition, SelectorPosition)> {
    let rows = diesel::sql_query(
        "SELECT kind, sn, token FROM outgoing_selector_cursors WHERE server_id = $1",
    )
    .bind::<Text, _>(server.as_str())
    .load::<CursorRow>(&mut crate::db())
    .await?;
    let mut device = SelectorPosition::complete(fallback);
    let mut receipt = device.clone();
    for row in rows {
        let position = SelectorPosition {
            sn: row.sn,
            token: row.token,
        };
        match row.kind.as_str() {
            "device" => device = position,
            "receipt" => receipt = position,
            _ => {}
        }
    }
    Ok((device, receipt))
}

pub async fn confirm_selector_position(
    conn: &mut AsyncPgConnection,
    server: &ServerName,
    kind: &str,
    position: &SelectorPosition,
) -> DataResult<()> {
    // Complete N sorts after every partial continuation at N. Never rewind a
    // committed position if another instance has already confirmed more work.
    diesel::sql_query(
        "INSERT INTO outgoing_selector_cursors (server_id,kind,sn,token) VALUES ($1,$2,$3,$4) \
         ON CONFLICT (server_id,kind) DO UPDATE SET sn=excluded.sn,token=excluded.token \
         WHERE excluded.sn > outgoing_selector_cursors.sn OR \
         (excluded.sn = outgoing_selector_cursors.sn AND outgoing_selector_cursors.token IS NOT NULL \
          AND (excluded.token IS NULL OR excluded.token > outgoing_selector_cursors.token))",
    )
    .bind::<Text, _>(server.as_str())
    .bind::<Text, _>(kind)
    .bind::<BigInt, _>(position.sn)
    .bind::<Nullable<Text>, _>(position.token.as_deref())
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Debug, QueryableByName)]
pub struct OutgoingDeviceChange {
    #[diesel(sql_type = Text)]
    pub user_id: OwnedUserId,
    #[diesel(sql_type = BigInt)]
    pub sn: i64,
    #[diesel(sql_type = BigInt)]
    pub stream_id: i64,
}

pub async fn outgoing_device_changes(
    server: &ServerName,
    local: &ServerName,
    position: &SelectorPosition,
    until: i64,
    limit: usize,
) -> DataResult<Vec<OutgoingDeviceChange>> {
    // Apply the window before grouping: a user with several room markers can
    // reappear at a newer position, but every tie has a finite continuation.
    diesel::sql_query(
        "SELECT c.user_id, c.sn, COALESCE((SELECT MAX(d.id) FROM device_streams d \
         WHERE d.user_id = c.user_id),0)::bigint AS stream_id FROM ( \
         SELECT k.user_id, MIN(k.occur_sn)::bigint AS sn FROM e2e_key_changes k \
         JOIN room_joined_servers r ON r.room_id = k.room_id AND r.server_id = $1 \
         WHERE substring(k.user_id FROM position(':' IN k.user_id) + 1) = $2 AND k.occur_sn <= $3 \
         AND (k.occur_sn > $4 OR ($5 IS NOT NULL AND k.occur_sn = $4 AND k.user_id > $5)) \
         GROUP BY k.user_id ORDER BY MIN(k.occur_sn), k.user_id LIMIT $6 \
         ) c ORDER BY c.sn,c.user_id",
    )
    .bind::<Text, _>(server.as_str())
    .bind::<Text, _>(local.as_str())
    .bind::<BigInt, _>(until)
    .bind::<BigInt, _>(position.sn)
    .bind::<Nullable<Text>, _>(position.token.as_deref())
    .bind::<BigInt, _>(limit as i64)
    .load(&mut crate::db())
    .await
    .map_err(Into::into)
}

#[derive(Debug, QueryableByName)]
pub struct OutgoingReceipt {
    #[diesel(sql_type = BigInt)]
    pub sn: i64,
    #[diesel(sql_type = Text)]
    pub room_id: OwnedRoomId,
    #[diesel(sql_type = Text)]
    pub user_id: OwnedUserId,
    #[diesel(sql_type = Text)]
    pub event_id: OwnedEventId,
    #[diesel(sql_type = Json)]
    pub json_data: serde_json::Value,
}

pub async fn outgoing_receipts(
    server: &ServerName,
    local: &ServerName,
    since: i64,
    until: i64,
    limit: usize,
) -> DataResult<Vec<OutgoingReceipt>> {
    // Receipt scopes are separate logical streams. Preserve main/threaded/
    // unthreaded identities and choose the newest event by SN, never event ID.
    diesel::sql_query(
        "SELECT sn,room_id,user_id,event_id,json_data FROM ( \
         SELECT DISTINCT ON (e.room_id,e.user_id,COALESCE(e.json_data->>'thread_id','')) \
         e.sn,e.room_id,e.user_id,e.event_id,e.json_data FROM event_receipts e \
         JOIN room_joined_servers r ON r.room_id = e.room_id AND r.server_id = $1 \
         WHERE e.ty = 'm.read' AND substring(e.user_id FROM position(':' IN e.user_id) + 1) = $2 \
         AND e.sn > $3 AND e.sn <= $4 \
         ORDER BY e.room_id,e.user_id,COALESCE(e.json_data->>'thread_id',''),e.sn DESC \
         ) latest ORDER BY sn LIMIT $5",
    )
    .bind::<Text, _>(server.as_str())
    .bind::<Text, _>(local.as_str())
    .bind::<BigInt, _>(since)
    .bind::<BigInt, _>(until)
    .bind::<BigInt, _>(limit as i64)
    .load(&mut crate::db())
    .await
    .map_err(Into::into)
}
