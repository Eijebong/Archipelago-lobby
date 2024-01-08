// @generated automatically by Diesel CLI.

diesel::table! {
    rooms (id) {
        id -> Binary,
        name -> Text,
        close_date -> Timestamp,
    }
}

diesel::table! {
    yamls (id) {
        id -> Binary,
        room_id -> Binary,
        owner_id -> Binary,
        content -> Text,
        player_name -> Text,
        game -> Text,
    }
}

diesel::allow_tables_to_appear_in_same_query!(rooms, yamls,);
