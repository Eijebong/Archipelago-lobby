// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "apworld"))]
    pub struct Apworld;

    #[derive(diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "validation_status"))]
    pub struct ValidationStatus;
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::db::types::sql::*;

    discord_users (id) {
        id -> Int8,
        username -> Varchar,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::db::types::sql::*;

    generations (room_id) {
        room_id -> SqlRoomId,
        job_id -> Uuid,
        status -> Varchar,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::db::types::sql::*;

    room_templates (id) {
        id -> SqlRoomTemplateId,
        name -> Varchar,
        close_date -> Timestamp,
        description -> Text,
        room_url -> Varchar,
        author_id -> Int8,
        yaml_validation -> Bool,
        allow_unsupported -> Bool,
        yaml_limit_per_user -> Nullable<Int4>,
        yaml_limit_bypass_list -> Array<Int8>,
        manifest -> Jsonb,
        show_apworlds -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        global -> Bool,
        tpl_name -> Varchar,
        allow_invalid_yamls -> Bool,
        meta_file -> Text,
        is_bundle_room -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::db::types::sql::*;

    rooms (id) {
        id -> SqlRoomId,
        name -> Varchar,
        close_date -> Timestamp,
        description -> Text,
        room_url -> Varchar,
        author_id -> Int8,
        yaml_validation -> Bool,
        allow_unsupported -> Bool,
        yaml_limit_per_user -> Nullable<Int4>,
        yaml_limit_bypass_list -> Array<Int8>,
        manifest -> Jsonb,
        show_apworlds -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        from_template_id -> Nullable<SqlRoomTemplateId>,
        allow_invalid_yamls -> Bool,
        meta_file -> Text,
        is_bundle_room -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::db::types::sql::*;
    use super::sql_types::ValidationStatus;
    use super::sql_types::Apworld;

    yamls (id) {
        id -> SqlYamlId,
        room_id -> SqlRoomId,
        content -> Text,
        player_name -> Varchar,
        game -> Varchar,
        owner_id -> Int8,
        features -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        validation_status -> ValidationStatus,
        apworlds -> Array<Apworld>,
        last_validation_time -> Timestamp,
        last_error -> Nullable<Text>,
        patch -> Nullable<Varchar>,
        bundle_id -> SqlBundleId,
        password -> Nullable<Varchar>,
    }
}

diesel::joinable!(generations -> rooms (room_id));
diesel::joinable!(room_templates -> discord_users (author_id));
diesel::joinable!(rooms -> discord_users (author_id));
diesel::joinable!(rooms -> room_templates (from_template_id));
diesel::joinable!(yamls -> discord_users (owner_id));
diesel::joinable!(yamls -> rooms (room_id));

diesel::allow_tables_to_appear_in_same_query!(
    discord_users,
    generations,
    room_templates,
    rooms,
    yamls,
);
