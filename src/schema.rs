// @generated automatically by Diesel CLI.

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
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use crate::db::types::sql::*;

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
    }
}

diesel::joinable!(room_templates -> discord_users (author_id));
diesel::joinable!(rooms -> discord_users (author_id));
diesel::joinable!(rooms -> room_templates (from_template_id));
diesel::joinable!(yamls -> discord_users (owner_id));
diesel::joinable!(yamls -> rooms (room_id));

diesel::allow_tables_to_appear_in_same_query!(discord_users, room_templates, rooms, yamls,);
