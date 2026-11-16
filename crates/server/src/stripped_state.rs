//! Opt-in MSC4560 queries. Preview data received from peers is never persisted.
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use salvo::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::Semaphore;

use crate::core::client::state::stripped_state::{
    StrippedStateMsc4560ReqArgs as ClientArgs, StrippedStateMsc4560ResBody as ClientResponse,
};
use crate::core::events::room::history_visibility::{
    HistoryVisibility, RoomHistoryVisibilityEventContent,
};
use crate::core::events::room::join_rule::RoomJoinRulesEventContent;
use crate::core::events::room::member::MembershipState;
use crate::core::events::room::server_acl::RoomServerAclEventContent;
use crate::core::events::{AnyStrippedStateEvent, StateEventType};
use crate::core::federation::event::stripped_state::{
    StrippedStateMsc4560ReqArgs as FederationArgs,
    StrippedStateMsc4560ResBody as FederationResponse,
};
use crate::core::federation::query::{RoomInfoResBody, directory_request};
use crate::core::room::JoinRule;
use crate::core::serde::RawJson;
use crate::core::stripped_state::{MAX_VIAS, StrippedStateMsc4560ReqBody as Selection};
use crate::core::{
    OwnedRoomId, OwnedServerName, OwnedUserId, RoomAliasId, RoomId, ServerName, UserId,
};
use crate::{
    AppResult, DepotExt, GetUrlOrigin, JsonResult, MatrixError, config, hoops, json_ok, room,
    sending,
};

const MAX_REQUEST: usize = 16 * 1024;
const MAX_RESPONSE: usize = 512 * 1024;
const MAX_EVENTS: usize = 1024;
const PEER_TIMEOUT: Duration = Duration::from_secs(2);
static REQUESTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(8)));

fn route(path: &str) -> Router {
    Router::with_path(path)
        .hoop(check_enabled)
        .hoop(salvo::size_limiter::max_size(MAX_REQUEST as u64))
        .hoop(salvo::http::request::SecureMaxSize(MAX_REQUEST))
        .hoop(admit)
        .hoop(hoops::limit_stripped_state)
}

pub(crate) fn client_router() -> Router {
    route("org.continuwuity.msc4560.stripped_state/{room_id_or_alias}")
        .hoop(hoops::optional_access_token)
        .post(client_query)
}

pub(crate) fn federation_router() -> Router {
    route("unstable/org.continuwuity.msc4560.stripped_state/{room_id}")
        .hoop(hoops::optional_server_signatures)
        .post(federation_query)
}

#[handler]
async fn check_enabled() -> AppResult<()> {
    if !config::get().enable_stripped_state {
        return Err(MatrixError::not_found("Stripped-state queries are disabled").into());
    }
    Ok(())
}

#[handler]
async fn admit(depot: &mut Depot) -> AppResult<()> {
    let permit = REQUESTS.clone().try_acquire_owned().map_err(|_| {
        let mut error = MatrixError::unknown("Stripped-state query capacity is exhausted");
        error.status_code = Some(StatusCode::SERVICE_UNAVAILABLE);
        error
    })?;
    depot.insert_typed(permit);
    Ok(())
}

async fn selection(req: &mut Request) -> AppResult<Selection> {
    let body: Selection = req
        .parse_json()
        .await
        .map_err(|_| MatrixError::bad_json("Invalid stripped-state request body"))?;
    body.validate()?;
    Ok(body)
}

#[handler]
async fn client_query(
    args: ClientArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> JsonResult<ClientResponse> {
    args.validate()?;
    let selection = selection(req).await?;
    // Both validations precede even alias resolution or destination discovery.
    let (room_id, vias) = resolve(args).await?;
    let sender = depot.authed_info().ok().map(|a| a.user_id().to_owned());
    if let Some(response) = local(&room_id, &selection, sender.as_deref(), None).await? {
        let response = ClientResponse {
            state: response.state,
            num_joined_members: response.num_joined_members,
            via: config::get().server_name.clone(),
        };
        bound(&response)?;
        return json_ok(response);
    }
    // Every supplied unique via is attempted before returning an error. Four
    // requests run at once; one peer's denial or malformed response cannot mask
    // another peer's successful response.
    let mut requests = stream::iter(vias)
        .map(|via| {
            let room_id = &room_id;
            let selection = &selection;
            async move { peer(&via, room_id, selection).await }
        })
        .buffer_unordered(4);
    while let Some(result) = requests.next().await {
        if let Ok(response) = result {
            return json_ok(response);
        }
    }
    Err(MatrixError::not_found("No via could provide the requested stripped state").into())
}

#[handler]
async fn federation_query(
    args: FederationArgs,
    req: &mut Request,
    depot: &mut Depot,
) -> JsonResult<FederationResponse> {
    let selection = selection(req).await?;
    let origin = depot.origin().ok().cloned();
    let response = local(&args.room_id, &selection, None, origin.as_deref())
        .await?
        .ok_or_else(|| MatrixError::not_found("This server is not resident in the room"))?;
    bound(&response)?;
    json_ok(response)
}

fn bound(value: &impl Serialize) -> AppResult<()> {
    if serde_json::to_vec(value)?.len() > MAX_RESPONSE {
        return Err(MatrixError::too_large("Stripped-state response exceeds 512 KiB").into());
    }
    Ok(())
}

async fn local(
    room_id: &RoomId,
    selection: &Selection,
    user: Option<&UserId>,
    server: Option<&ServerName>,
) -> AppResult<Option<FederationResponse>> {
    let _guard = room::lock_state(room_id).await;
    if crate::data::room::is_disabled(room_id).await? {
        return Err(MatrixError::forbidden("Room is disabled", None).into());
    }
    if !room::is_server_joined(&config::get().server_name, room_id).await? {
        return Ok(None);
    }
    let frame = room::get_frame_id(room_id, None).await?;
    authorize(frame, room_id, user, server).await?;
    let mut response = FederationResponse::default();
    let mut bytes = 0_usize;
    for pdu in room::state::get_state_for_types(frame, &selection.state, MAX_EVENTS).await? {
        let event_type: StateEventType = pdu.event_ty.to_string().into();
        if pdu.room_id != room_id || !selection.state.contains(&event_type) {
            return Err(MatrixError::unknown("Invalid selected room state").into());
        }
        let event = SafeState {
            event_type,
            state_key: pdu
                .state_key
                .clone()
                .ok_or_else(|| MatrixError::unknown("Selected event is not state"))?,
            sender: pdu.sender.clone(),
            content: serde_json::from_str(pdu.content.get())?,
        };
        let event = RawJson::new(&event)?;
        bytes = bytes.saturating_add(event.as_str().len() + 1);
        if bytes > MAX_RESPONSE {
            return Err(MatrixError::too_large("Stripped state exceeds 512 KiB").into());
        }
        response.state.push(event.cast());
    }
    if selection.num_joined_members {
        response.num_joined_members = Some(room::joined_member_count(room_id).await?);
    }
    Ok(Some(response))
}

async fn authorize(
    frame: i64,
    room_id: &RoomId,
    user: Option<&UserId>,
    server: Option<&ServerName>,
) -> AppResult<()> {
    use room::state::get_state_content;
    if let Some(server) = server {
        match get_state_content::<RoomServerAclEventContent>(
            frame,
            &StateEventType::RoomServerAcl,
            "",
        )
        .await
        {
            Ok(acl) if !acl.is_allowed(server) => {
                return Err(
                    MatrixError::forbidden("Server is denied by the room ACL", None).into(),
                );
            }
            Ok(_) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error),
        }
        if room::is_server_joined(server, room_id).await? {
            return Ok(());
        }
    }
    match get_state_content::<RoomHistoryVisibilityEventContent>(
        frame,
        &StateEventType::RoomHistoryVisibility,
        "",
    )
    .await
    {
        Ok(visibility) if visibility.history_visibility == HistoryVisibility::WorldReadable => {
            return Ok(());
        }
        Ok(_) => {}
        Err(error) if error.is_not_found() => {} // The protocol default is shared.
        Err(error) => return Err(error),
    }
    if let Some(user) = user {
        match room::state::user_membership(frame, user).await {
            Ok(MembershipState::Join | MembershipState::Invite) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error),
        }
    }
    let rule = match get_state_content::<RoomJoinRulesEventContent>(
        frame,
        &StateEventType::RoomJoinRules,
        "",
    )
    .await
    {
        Ok(rules) => rules.join_rule,
        Err(error) if error.is_not_found() => JoinRule::Invite,
        Err(error) => return Err(error),
    };
    match &rule {
        JoinRule::Public | JoinRule::Knock | JoinRule::KnockRestricted(_) => return Ok(()),
        JoinRule::Restricted(_) => {
            for allowed in rule.allowed_rooms() {
                if let Some(user) = user
                    && room::user::is_joined(user, allowed).await?
                {
                    return Ok(());
                }
                if let Some(server) = server
                    && room::is_server_joined(server, allowed).await?
                {
                    return Ok(());
                }
            }
        }
        _ => {}
    }
    Err(MatrixError::forbidden("Requester cannot view this room's stripped state", None).into())
}

#[derive(Deserialize, Serialize)]
struct SafeState {
    #[serde(rename = "type")]
    event_type: StateEventType,
    state_key: String,
    sender: OwnedUserId,
    content: Map<String, Value>,
}

fn sanitize(
    response: FederationResponse,
    selection: &Selection,
    via: OwnedServerName,
) -> AppResult<ClientResponse> {
    if response.state.len() > MAX_EVENTS {
        return Err(MatrixError::too_large("Too many stripped state events").into());
    }
    let mut seen = HashSet::new();
    let mut state: Vec<RawJson<AnyStrippedStateEvent>> = Vec::new();
    for raw in response.state {
        let event: SafeState = serde_json::from_str(raw.as_str())?;
        if !selection.state.contains(&event.event_type)
            || !seen.insert((event.event_type.clone(), event.state_key.clone()))
        {
            return Err(
                MatrixError::invalid_param("Unexpected or duplicate stripped state").into(),
            );
        }
        state.push(RawJson::new(&event)?.cast());
    }
    let count = if selection.num_joined_members {
        Some(
            response
                .num_joined_members
                .ok_or_else(|| MatrixError::invalid_param("Missing requested member count"))?,
        )
    } else {
        None
    };
    let response = ClientResponse {
        state,
        num_joined_members: count,
        via,
    };
    bound(&response)?;
    Ok(response)
}

async fn read_limited(mut response: reqwest::Response, limit: usize) -> AppResult<Vec<u8>> {
    if !response.status().is_success() {
        return Err(MatrixError::not_found("Peer denied the request").into());
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(MatrixError::too_large("Peer response is oversized").into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(MatrixError::too_large("Peer response is oversized").into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn permitted(server: &ServerName) -> AppResult<()> {
    if server == config::get().server_name
        || config::get().enabled_federation().is_none()
        || !config::get().federation.is_server_allowed(server)
    {
        return Err(MatrixError::not_found("Federation destination is disabled").into());
    }
    Ok(())
}

async fn peer(
    server: &ServerName,
    room_id: &RoomId,
    selection: &Selection,
) -> AppResult<ClientResponse> {
    permitted(server)?;
    tokio::time::timeout(PEER_TIMEOUT, async {
        let origin = server.origin().await;
        let mut url = reqwest::Url::parse(&format!(
            "{origin}/_matrix/federation/unstable/org.continuwuity.msc4560.stripped_state/"
        ))?;
        url.path_segments_mut()
            .map_err(|_| MatrixError::invalid_param("Invalid peer origin"))?
            .pop_if_empty()
            .push(room_id.as_str());
        let request = crate::core::sending::post(url)
            .stuff(selection.clone())?
            .into_inner();
        let response = sending::send_federation_request(server, request, Some(2)).await?;
        let body = read_limited(response, MAX_RESPONSE).await?;
        sanitize(serde_json::from_slice(&body)?, selection, server.to_owned())
    })
    .await
    .map_err(|_| MatrixError::not_found("Stripped-state peer deadline exceeded"))?
}

async fn resolve(args: ClientArgs) -> AppResult<(OwnedRoomId, Vec<OwnedServerName>)> {
    let room_or_alias: &crate::core::RoomOrAliasId = args.room_id_or_alias.as_ref();
    let mut vias = Vec::new();
    for via in args.via {
        if !vias.contains(&via) {
            vias.push(via);
        }
    }
    if args.room_id_or_alias.is_room_id() {
        let room: &RoomId = room_or_alias.try_into().expect("room ID");
        return Ok((room.to_owned(), vias));
    }
    let alias: &RoomAliasId = room_or_alias.try_into().expect("room alias");
    if alias.server_name() == config::get().server_name {
        return Ok((room::alias::resolve_local_alias(alias).await?, vias));
    }
    let server = alias.server_name();
    permitted(server)?;
    let info: RoomInfoResBody = tokio::time::timeout(PEER_TIMEOUT, async {
        let request = directory_request(&server.origin().await, alias)?.into_inner();
        let response = sending::send_federation_request(server, request, Some(2)).await?;
        let body = read_limited(response, 64 * 1024).await?;
        Ok::<_, crate::AppError>(serde_json::from_slice(&body)?)
    })
    .await
    .map_err(|_| MatrixError::not_found("Alias lookup deadline exceeded"))??;
    for via in info
        .servers
        .into_iter()
        .chain(std::iter::once(server.to_owned()))
    {
        if vias.len() == MAX_VIAS {
            break;
        }
        if !vias.contains(&via) {
            vias.push(via);
        }
    }
    Ok((info.room_id, vias))
}

#[cfg(test)]
mod tests;
