// @generated automatically by Diesel CLI.

diesel::table! {
    discord_users (id) {
        id -> Int8,
        username -> Varchar,
    }
}

diesel::table! {
    review_presets (id) {
        id -> Int4,
        name -> Text,
        builtin_rules -> Jsonb,
    }
}

diesel::table! {
    review_preset_rules (id) {
        id -> Int4,
        preset_id -> Int4,
        rule -> Jsonb,
        position -> Int4,
        last_edited_by -> Nullable<Int8>,
        last_edited_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    room_review_config (room_id) {
        room_id -> Uuid,
        preset_id -> Int4,
    }
}

diesel::table! {
    yaml_review_status (room_id, yaml_id) {
        room_id -> Uuid,
        yaml_id -> Uuid,
        status -> Text,
        changed_by -> Int8,
        changed_at -> Timestamptz,
    }
}

diesel::joinable!(review_preset_rules -> review_presets (preset_id));
diesel::joinable!(room_review_config -> review_presets (preset_id));
diesel::allow_tables_to_appear_in_same_query!(discord_users, review_presets, review_preset_rules, room_review_config, yaml_review_status);
