// @generated automatically by Diesel CLI.

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    appservice_registrations (id) {
        id -> Text,
        url -> Nullable<Text>,
        as_token -> Text,
        hs_token -> Text,
        sender_localpart -> Text,
        namespaces -> Json,
        rate_limited -> Nullable<Bool>,
        protocols -> Nullable<Json>,
        receive_ephemeral -> Bool,
        device_management -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    banned_rooms (id) {
        id -> Int8,
        room_id -> Text,
        created_by -> Nullable<Text>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    device_inboxes (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        json_data -> Json,
        occur_sn -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    device_streams (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_cross_signing_keys (id) {
        id -> Int8,
        user_id -> Text,
        key_type -> Text,
        key_data -> Json,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_cross_signing_sigs (id) {
        id -> Int8,
        origin_user_id -> Text,
        origin_key_id -> Text,
        target_user_id -> Text,
        target_device_id -> Text,
        signature -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_cross_signing_uia_bypass (user_id) {
        user_id -> Text,
        updatable_before_ts -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_device_keys (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        stream_id -> Int8,
        display_name -> Nullable<Text>,
        key_data -> Json,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_fallback_keys (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        algorithm -> Text,
        key_id -> Text,
        key_data -> Json,
        used_at -> Nullable<Int8>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_key_changes (id) {
        id -> Int8,
        user_id -> Text,
        room_id -> Nullable<Text>,
        occur_sn -> Int8,
        changed_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_one_time_keys (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        algorithm -> Text,
        key_id -> Text,
        key_data -> Json,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_room_keys (id) {
        id -> Int8,
        user_id -> Text,
        room_id -> Text,
        session_id -> Text,
        version -> Int8,
        first_message_index -> Nullable<Int8>,
        forwarded_count -> Nullable<Int8>,
        is_verified -> Bool,
        session_data -> Json,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    e2e_room_keys_versions (id) {
        id -> Int8,
        user_id -> Text,
        version -> Int8,
        algorithm -> Json,
        auth_data -> Json,
        is_trashed -> Bool,
        etag -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_auth_chains (cache_key) {
        cache_key -> Array<Nullable<Int8>>,
        chain_sns -> Array<Nullable<Int8>>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_backward_extremities (id) {
        id -> Int8,
        event_id -> Text,
        room_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_datas (event_id) {
        event_id -> Text,
        event_sn -> Int8,
        room_id -> Text,
        internal_metadata -> Nullable<Json>,
        format_version -> Nullable<Int8>,
        json_data -> Json,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_edges (id) {
        id -> Int8,
        room_id -> Text,
        event_id -> Text,
        event_sn -> Int8,
        event_depth -> Int8,
        prev_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_forward_extremities (id) {
        id -> Int8,
        event_id -> Text,
        room_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_idempotents (id) {
        id -> Int8,
        txn_id -> Text,
        user_id -> Text,
        device_id -> Nullable<Text>,
        room_id -> Nullable<Text>,
        event_id -> Nullable<Text>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_missings (id) {
        id -> Int4,
        room_id -> Text,
        event_id -> Text,
        event_sn -> Int8,
        missing_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_phases (event_id) {
        event_id -> Text,
        curr -> Text,
        next -> Text,
        goal -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_points (event_id) {
        event_id -> Text,
        event_sn -> Int8,
        room_id -> Text,
        thread_id -> Nullable<Text>,
        frame_id -> Nullable<Int8>,
        stripped_state -> Nullable<Json>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_push_actions (id) {
        id -> Int8,
        room_id -> Text,
        event_id -> Text,
        event_sn -> Int8,
        user_id -> Text,
        profile_tag -> Text,
        actions -> Jsonb,
        topological_ordering -> Nullable<Int8>,
        stream_ordering -> Nullable<Int8>,
        notify -> Bool,
        highlight -> Bool,
        unread -> Bool,
        thread_id -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_push_summaries (id) {
        id -> Int8,
        user_id -> Text,
        room_id -> Text,
        notification_count -> Int8,
        highlight_count -> Int8,
        unread_count -> Int8,
        stream_ordering -> Int8,
        thread_id -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_receipts (sn) {
        sn -> Int8,
        ty -> Text,
        room_id -> Text,
        user_id -> Text,
        event_id -> Text,
        event_sn -> Int8,
        thread_id -> Nullable<Text>,
        json_data -> Json,
        receipt_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_relations (id) {
        id -> Int8,
        room_id -> Text,
        event_id -> Text,
        event_sn -> Int8,
        event_ty -> Text,
        child_id -> Text,
        child_sn -> Int8,
        child_ty -> Text,
        rel_type -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_reports (id) {
        id -> Int8,
        received_ts -> Int8,
        room_id -> Text,
        event_id -> Text,
        user_id -> Text,
        reason -> Nullable<Text>,
        content -> Nullable<Jsonb>,
        score -> Nullable<Int8>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    event_searches (id) {
        id -> Int8,
        event_id -> Text,
        event_sn -> Int8,
        room_id -> Text,
        sender_id -> Text,
        key -> Text,
        vector -> Tsvector,
        origin_server_ts -> Int8,
        stream_ordering -> Nullable<Int8>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    events (id) {
        id -> Text,
        sn -> Int8,
        ty -> Text,
        room_id -> Text,
        depth -> Int8,
        topological_ordering -> Int8,
        stream_ordering -> Int8,
        unrecognized_keys -> Nullable<Text>,
        origin_server_ts -> Int8,
        received_at -> Nullable<Int8>,
        sender_id -> Nullable<Text>,
        contains_url -> Bool,
        worker_id -> Nullable<Text>,
        state_key -> Nullable<Text>,
        is_outlier -> Bool,
        is_redacted -> Bool,
        soft_failed -> Bool,
        is_rejected -> Bool,
        rejection_reason -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    lazy_load_deliveries (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        room_id -> Text,
        confirmed_user_id -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    media_metadatas (id) {
        id -> Int8,
        media_id -> Text,
        origin_server -> Text,
        content_type -> Nullable<Text>,
        disposition_type -> Nullable<Text>,
        file_name -> Nullable<Text>,
        file_extension -> Nullable<Text>,
        file_size -> Int8,
        file_hash -> Nullable<Text>,
        created_by -> Nullable<Text>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    media_thumbnails (id) {
        id -> Int8,
        media_id -> Text,
        origin_server -> Text,
        content_type -> Nullable<Text>,
        disposition_type -> Nullable<Text>,
        file_size -> Int8,
        width -> Int4,
        height -> Int4,
        resize_method -> Text,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    media_url_previews (id) {
        id -> Int8,
        url -> Text,
        og_title -> Nullable<Text>,
        og_type -> Nullable<Text>,
        og_url -> Nullable<Text>,
        og_description -> Nullable<Text>,
        og_image -> Nullable<Text>,
        image_size -> Nullable<Int8>,
        og_image_width -> Nullable<Int4>,
        og_image_height -> Nullable<Int4>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    outgoing_requests (id) {
        id -> Int8,
        kind -> Text,
        appservice_id -> Nullable<Text>,
        user_id -> Nullable<Text>,
        pushkey -> Nullable<Text>,
        server_id -> Nullable<Text>,
        pdu_id -> Nullable<Text>,
        edu_json -> Nullable<Bytea>,
        state -> Text,
        data -> Nullable<Bytea>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_aliases (alias_id) {
        alias_id -> Text,
        room_id -> Text,
        created_by -> Text,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_joined_servers (id) {
        id -> Int8,
        room_id -> Text,
        server_id -> Text,
        occur_sn -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_lookup_servers (id) {
        id -> Int8,
        room_id -> Text,
        alias_id -> Text,
        server_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_state_deltas (frame_id) {
        frame_id -> Int8,
        room_id -> Text,
        parent_id -> Nullable<Int8>,
        appended -> Bytea,
        disposed -> Bytea,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_state_fields (id) {
        id -> Int8,
        event_ty -> Text,
        state_key -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_state_frames (id) {
        id -> Int8,
        room_id -> Text,
        hash_data -> Bytea,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_tags (id) {
        id -> Int8,
        user_id -> Text,
        room_id -> Text,
        tag -> Text,
        content -> Json,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    room_users (id) {
        id -> Int8,
        event_id -> Text,
        event_sn -> Int8,
        room_id -> Text,
        room_server_id -> Nullable<Text>,
        user_id -> Text,
        user_server_id -> Text,
        sender_id -> Text,
        membership -> Text,
        forgotten -> Bool,
        display_name -> Nullable<Text>,
        avatar_url -> Nullable<Text>,
        state_data -> Nullable<Json>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    rooms (id) {
        id -> Text,
        sn -> Int8,
        version -> Text,
        is_public -> Bool,
        min_depth -> Int8,
        state_frame_id -> Nullable<Int8>,
        has_auth_chain_index -> Bool,
        disabled -> Bool,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    server_signing_keys (server_id) {
        server_id -> Text,
        key_data -> Json,
        updated_at -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    stats_monthly_active_users (id) {
        id -> Int8,
        user_id -> Text,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    stats_room_currents (room_id) {
        room_id -> Text,
        state_events -> Int8,
        joined_members -> Int8,
        invited_members -> Int8,
        left_members -> Int8,
        banned_members -> Int8,
        knocked_members -> Int8,
        local_users_in_room -> Int8,
        completed_delta_stream_id -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    stats_user_daily_visits (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        user_agent -> Nullable<Text>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    threads (event_id) {
        event_id -> Text,
        event_sn -> Int8,
        room_id -> Text,
        last_id -> Text,
        last_sn -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    threepid_guests (id) {
        id -> Int8,
        medium -> Nullable<Text>,
        address -> Nullable<Text>,
        access_token -> Nullable<Text>,
        first_inviter -> Nullable<Text>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    threepid_id_servers (id) {
        id -> Int8,
        user_id -> Text,
        medium -> Text,
        address -> Text,
        id_server -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    threepid_validation_sessions (id) {
        id -> Int8,
        session_id -> Text,
        medium -> Text,
        address -> Text,
        client_secret -> Text,
        last_send_attempt -> Int8,
        validated_at -> Nullable<Int8>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    threepid_validation_tokens (id) {
        id -> Int8,
        token -> Text,
        session_id -> Text,
        next_link -> Nullable<Text>,
        expires_at -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    timeline_gaps (id) {
        id -> Int8,
        room_id -> Text,
        event_sn -> Int8,
        event_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_access_tokens (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        token -> Text,
        puppets_user_id -> Nullable<Text>,
        last_validated -> Nullable<Int8>,
        refresh_token_id -> Nullable<Int8>,
        is_used -> Bool,
        expires_at -> Nullable<Int8>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_datas (id) {
        id -> Int8,
        user_id -> Text,
        room_id -> Nullable<Text>,
        data_type -> Text,
        json_data -> Json,
        occur_sn -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_dehydrated_devices (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        device_data -> Json,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_devices (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        display_name -> Nullable<Text>,
        user_agent -> Nullable<Text>,
        is_hidden -> Bool,
        last_seen_ip -> Nullable<Text>,
        last_seen_at -> Nullable<Int8>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_external_ids (id) {
        id -> Int8,
        auth_provider -> Text,
        external_id -> Text,
        user_id -> Text,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_filters (id) {
        id -> Int8,
        user_id -> Text,
        filter -> Json,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_ignores (id) {
        id -> Int8,
        user_id -> Text,
        ignored_id -> Text,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_login_tokens (id) {
        id -> Int8,
        user_id -> Text,
        token -> Text,
        expires_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_openid_tokens (id) {
        id -> Int8,
        user_id -> Text,
        token -> Text,
        expires_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_passwords (id) {
        id -> Int8,
        user_id -> Text,
        hash -> Text,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_presences (id) {
        id -> Int8,
        user_id -> Text,
        stream_id -> Nullable<Int8>,
        state -> Nullable<Text>,
        status_msg -> Nullable<Text>,
        last_active_at -> Nullable<Int8>,
        last_federation_update_at -> Nullable<Int8>,
        last_user_sync_at -> Nullable<Int8>,
        currently_active -> Nullable<Bool>,
        occur_sn -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_profiles (id) {
        id -> Int8,
        user_id -> Text,
        room_id -> Nullable<Text>,
        display_name -> Nullable<Text>,
        avatar_url -> Nullable<Text>,
        blurhash -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_pushers (id) {
        id -> Int8,
        user_id -> Text,
        kind -> Text,
        app_id -> Text,
        app_display_name -> Text,
        device_id -> Text,
        device_display_name -> Text,
        access_token_id -> Nullable<Int8>,
        profile_tag -> Nullable<Text>,
        pushkey -> Text,
        lang -> Text,
        data -> Json,
        enabled -> Bool,
        last_stream_ordering -> Nullable<Int8>,
        last_success -> Nullable<Int8>,
        failing_since -> Nullable<Int8>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_ratelimit_override (user_id) {
        user_id -> Text,
        messages_per_second -> Nullable<Int4>,
        burst_count -> Nullable<Int4>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_refresh_tokens (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        token -> Text,
        next_token_id -> Nullable<Int8>,
        expires_at -> Int8,
        ultimate_session_expires_at -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_registration_tokens (id) {
        id -> Int8,
        token -> Text,
        uses_allowed -> Nullable<Int8>,
        pending -> Int8,
        completed -> Int8,
        expires_at -> Nullable<Int8>,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_sessions (id) {
        id -> Int8,
        user_id -> Text,
        session_id -> Text,
        session_type -> Text,
        value -> Json,
        expires_at -> Int8,
        created_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_threepids (id) {
        id -> Int8,
        user_id -> Text,
        medium -> Text,
        address -> Text,
        validated_at -> Int8,
        added_at -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    user_uiaa_datas (id) {
        id -> Int8,
        user_id -> Text,
        device_id -> Text,
        session -> Text,
        uiaa_info -> Json,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::full_text_search::*;

    users (id) {
        id -> Text,
        ty -> Nullable<Text>,
        is_admin -> Bool,
        is_guest -> Bool,
        is_local -> Bool,
        localpart -> Text,
        server_name -> Text,
        appservice_id -> Nullable<Text>,
        shadow_banned -> Bool,
        consent_at -> Nullable<Int8>,
        consent_version -> Nullable<Text>,
        consent_server_notice_sent -> Nullable<Text>,
        approved_at -> Nullable<Int8>,
        approved_by -> Nullable<Text>,
        deactivated_at -> Nullable<Int8>,
        deactivated_by -> Nullable<Text>,
        locked_at -> Nullable<Int8>,
        locked_by -> Nullable<Text>,
        created_at -> Int8,
        suspended_at -> Nullable<Int8>,
    }
}

diesel::joinable!(e2e_cross_signing_uia_bypass -> users (user_id));
diesel::joinable!(user_external_ids -> users (user_id));
diesel::joinable!(user_ratelimit_override -> users (user_id));

diesel::allow_tables_to_appear_in_same_query!(
    appservice_registrations,
    banned_rooms,
    device_inboxes,
    device_streams,
    e2e_cross_signing_keys,
    e2e_cross_signing_sigs,
    e2e_cross_signing_uia_bypass,
    e2e_device_keys,
    e2e_fallback_keys,
    e2e_key_changes,
    e2e_one_time_keys,
    e2e_room_keys,
    e2e_room_keys_versions,
    event_auth_chains,
    event_backward_extremities,
    event_datas,
    event_edges,
    event_forward_extremities,
    event_idempotents,
    event_missings,
    event_phases,
    event_points,
    event_reports,
    event_push_actions,
    event_push_summaries,
    event_receipts,
    event_relations,
    event_searches,
    events,
    lazy_load_deliveries,
    media_metadatas,
    media_thumbnails,
    media_url_previews,
    outgoing_requests,
    room_aliases,
    room_joined_servers,
    room_lookup_servers,
    room_state_deltas,
    room_state_fields,
    room_state_frames,
    room_tags,
    room_users,
    rooms,
    server_signing_keys,
    stats_monthly_active_users,
    stats_room_currents,
    stats_user_daily_visits,
    threads,
    threepid_guests,
    threepid_id_servers,
    threepid_validation_sessions,
    threepid_validation_tokens,
    timeline_gaps,
    user_access_tokens,
    user_datas,
    user_dehydrated_devices,
    user_devices,
    user_external_ids,
    user_filters,
    user_ignores,
    user_login_tokens,
    user_openid_tokens,
    user_passwords,
    user_presences,
    user_profiles,
    user_pushers,
    user_ratelimit_override,
    user_refresh_tokens,
    user_registration_tokens,
    user_sessions,
    user_threepids,
    user_uiaa_datas,
    users,
);
