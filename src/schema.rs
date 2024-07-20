// @generated automatically by Diesel CLI.

diesel::table! {
    discord_users (id) {
        id -> Int8,
        username -> Varchar,
    }
}

diesel::table! {
    rooms (id) {
        id -> Uuid,
        name -> Varchar,
        close_date -> Timestamp,
        description -> Text,
        room_url -> Varchar,
        author_id -> Int8,
        private -> Bool,
        yaml_validation -> Bool,
        allow_unsupported -> Bool,
    }
}

diesel::table! {
    yamls (id) {
        id -> Uuid,
        room_id -> Uuid,
        content -> Text,
        player_name -> Varchar,
        game -> Varchar,
        owner_id -> Int8,
    }
}

diesel::joinable!(rooms -> discord_users (author_id));
diesel::joinable!(yamls -> discord_users (owner_id));
diesel::joinable!(yamls -> rooms (room_id));

diesel::allow_tables_to_appear_in_same_query!(
    discord_users,
    rooms,
    yamls,
);
