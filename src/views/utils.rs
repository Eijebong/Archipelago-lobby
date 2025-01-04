use ap_lobby::db::Room;

pub fn partition_rooms_by_closed(rooms: &Vec<Room>) -> (Vec<&Room>, Vec<&Room>) {
    rooms.iter().partition(|room| room.is_closed())
}
