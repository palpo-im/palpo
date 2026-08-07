mod account;
mod filter;
mod openid;
mod room;

use salvo::prelude::*;

use crate::hoops;

pub fn authed_router() -> Router {
    Router::with_path("user")
        .push(
            Router::with_hoop(hoops::limit_rate).push(
                Router::with_path("{user_id}")
                    .push(Router::with_path("mutual_rooms").get(room::get_mutual_rooms))
                    .push(Router::with_path("openid/request_token").post(openid::request_token)),
            ),
        )
        .push(
            Router::with_path("{user_id}")
                .push(
                    Router::with_path("filter")
                        .post(filter::create_filter)
                        .push(Router::with_path("{filter_id}").get(filter::get_filter)),
                )
                .push(
                    Router::with_path("account_data/{event_type}")
                        .get(account::get_global_data)
                        .put(account::set_global_data),
                )
                .push(
                    Router::with_path("rooms/{room_id}/account_data/{event_type}")
                        .get(account::get_room_data)
                        .put(account::set_room_data),
                ),
        )
}

pub fn stable_v1_router() -> Router {
    Router::with_hoop(hoops::limit_rate).push(
        Router::with_path("mutual_rooms")
            .hoop(hoops::auth_by_access_token_without_query_masquerade)
            .get(room::get_mutual_rooms_v1),
    )
}
