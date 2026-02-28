// @generated automatically by Diesel CLI.

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
        last_edited_by_name -> Nullable<Text>,
    }
}

diesel::table! {
    room_review_config (room_id) {
        room_id -> Uuid,
        preset_id -> Int4,
    }
}

diesel::table! {
    yaml_review_notes (id) {
        id -> Int4,
        room_id -> Uuid,
        yaml_id -> Uuid,
        content -> Text,
        author_id -> Int8,
        author_name -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    yaml_review_status (room_id, yaml_id) {
        room_id -> Uuid,
        yaml_id -> Uuid,
        status -> Text,
        changed_by -> Int8,
        changed_at -> Timestamptz,
        changed_by_name -> Nullable<Text>,
    }
}

diesel::joinable!(review_preset_rules -> review_presets (preset_id));
diesel::joinable!(room_review_config -> review_presets (preset_id));
diesel::allow_tables_to_appear_in_same_query!(
    review_presets,
    review_preset_rules,
    room_review_config,
    yaml_review_notes,
    yaml_review_status
);
