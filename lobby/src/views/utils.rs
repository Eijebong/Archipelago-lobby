use crate::db::Room;

pub fn partition_rooms_by_closed(rooms: &[Room]) -> (Vec<&Room>, Vec<&Room>) {
    rooms.iter().partition(|room| room.is_closed())
}
