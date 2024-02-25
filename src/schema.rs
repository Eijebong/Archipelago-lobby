// @generated automatically by Diesel CLI.

diesel::table! {
    discord_users (id) {
        id -> BigInt,
        username -> Text,
    }
}

diesel::table! {
    rooms (id) {
        id -> Binary,
        name -> Text,
        close_date -> Timestamp,
        description -> Text,
    }
}

diesel::table! {
    yamls (id) {
        id -> Binary,
        room_id -> Binary,
        content -> Text,
        player_name -> Text,
        game -> Text,
        owner_id -> BigInt,
    }
}

diesel::allow_tables_to_appear_in_same_query!(discord_users, rooms, yamls,);
