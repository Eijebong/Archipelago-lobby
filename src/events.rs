use crate::db::{RoomId, YamlId, YamlValidationStatus};
use crate::error::Result;
use async_stream::stream;
use futures_core::stream::Stream;
use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "ty")]
pub enum RoomEventTy {
    YamlValidationStatusChanged {
        yaml_id: YamlId,
        new_status: YamlValidationStatus,
        new_error: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct RoomEvent(RoomId, RoomEventTy);

pub struct RoomEventsReceiver(broadcast::Receiver<RoomEvent>);
#[derive(Clone)]
pub struct RoomEventsSender(broadcast::Sender<RoomEvent>);

impl RoomEventsReceiver {
    pub fn stream_for_room(&self, room_id: RoomId) -> impl Stream<Item = RoomEventTy> {
        let mut rx = self.0.resubscribe();

        stream! {
            while let Ok(msg) = rx.recv().await {
                if msg.0 != room_id {
                    continue
                }
                yield msg.1
            }
        }
    }
}

impl RoomEventsSender {
    pub async fn send_event(&self, room_id: RoomId, event: RoomEventTy) -> Result<()> {
        self.0.send(RoomEvent(room_id, event))?;
        Ok(())
    }
}

pub fn room_events() -> (RoomEventsReceiver, RoomEventsSender) {
    let (tx, rx) = broadcast::channel::<RoomEvent>(10);

    (RoomEventsReceiver(rx), RoomEventsSender(tx))
}
