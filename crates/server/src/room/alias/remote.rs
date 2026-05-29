use crate::core::federation::query::{RoomInfoResBody, directory_request};
use crate::core::identifiers::*;
use crate::{AppError, AppResult, GetUrlOrigin, MatrixError};

pub(super) async fn remote_resolve(
    room_alias: &RoomAliasId,
    servers: Vec<OwnedServerName>,
) -> AppResult<(OwnedRoomId, Vec<OwnedServerName>)> {
    debug!(?room_alias, servers = ?servers, "remote resolve");
    let servers = [vec![room_alias.server_name().to_owned()], servers].concat();

    let mut resolved_servers = Vec::new();
    let mut resolved_room_id: Option<OwnedRoomId> = None;
    let mut last_error: Option<AppError> = None;
    for server in servers {
        match remote_request(room_alias, &server).await {
            Err(e) => {
                tracing::error!("Failed to query for {room_alias:?} from {server}: {e}");
                // Keep the most informative failure across candidate servers: a
                // transport/signature/internal error from one server must not be
                // overwritten by a later not-found, otherwise the client path would
                // re-mask it as "Room with alias not found" depending on server order.
                if !e.is_not_found() || last_error.as_ref().is_none_or(AppError::is_not_found) {
                    last_error = Some(e);
                }
            }
            Ok(RoomInfoResBody { room_id, servers }) => {
                debug!(
                    "Server {server} answered with {room_id:?} for {room_alias:?} servers: \
					 {servers:?}"
                );
                resolved_room_id.get_or_insert(room_id);
                add_server(&mut resolved_servers, server);

                if !servers.is_empty() {
                    add_servers(&mut resolved_servers, servers);
                    break;
                }
            }
        }
    }

    match resolved_room_id {
        Some(room_id) => Ok((room_id, resolved_servers)),
        // Surface the actual failure from the last queried server (e.g. a transport or
        // signature error) so it is not masked as a plain "not found". Only fall back to
        // the generic message when no server produced an error to report.
        None => Err(last_error.unwrap_or_else(|| {
            MatrixError::not_found("No servers could assist in resolving the room alias").into()
        })),
    }
}

async fn remote_request(
    room_alias: &RoomAliasId,
    server: &ServerName,
) -> AppResult<RoomInfoResBody> {
    let request = directory_request(&server.origin().await, room_alias)?.into_inner();
    crate::sending::send_federation_request(server, request, None)
        .await?
        .json::<RoomInfoResBody>()
        .await
        .map_err(Into::into)
}

fn add_servers(servers: &mut Vec<OwnedServerName>, new: Vec<OwnedServerName>) {
    for server in new {
        add_server(servers, server);
    }
}

fn add_server(servers: &mut Vec<OwnedServerName>, server: OwnedServerName) {
    if !servers.contains(&server) {
        servers.push(server);
    }
}
