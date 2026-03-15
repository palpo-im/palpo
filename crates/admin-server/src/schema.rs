// Admin-server's own database schema.
// Palpo Matrix data is accessed via HTTP API (PalpoClient), NOT direct DB.

diesel::table! {
    webui_admins (id) {
        id -> Int8,
        username -> Text,
        password_hash -> Text,
        display_name -> Nullable<Text>,
        is_active -> Bool,
        created_at -> Timestamptz,
        last_login_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    audit_logs (id) {
        id -> Int8,
        admin_id -> Nullable<Int8>,
        action -> Text,
        target -> Nullable<Text>,
        details -> Nullable<Jsonb>,
        ip_address -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::joinable!(audit_logs -> webui_admins (admin_id));

diesel::allow_tables_to_appear_in_same_query!(webui_admins, audit_logs);
