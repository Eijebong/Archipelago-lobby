use ap_lobby::db::{Room, RoomSettings};
use ap_lobby::error::Result;
use apwm::Manifest;
use askama::Template;
use uuid::Uuid;

use crate::TplContext;

use super::manifest_editor::ManifestFormBuilder;

#[derive(Template)]
#[template(path = "room_manager/room_form.html")]
pub struct RoomSettingsBuilder<'a> {
    base: TplContext<'a>,
    room: RoomSettings,
    manifest_builder: ManifestFormBuilder,
    room_id: Option<Uuid>,
}

impl<'a> RoomSettingsBuilder<'a> {
    pub fn new_with_room(
        base: TplContext<'a>,
        index: apwm::Index,
        room: Room,
    ) -> RoomSettingsBuilder<'a> {
        Self {
            base,
            manifest_builder: ManifestFormBuilder::new(index, room.settings.manifest.0.clone()),
            room: room.settings,
            room_id: Some(room.id),
        }
    }

    pub fn new(base: TplContext<'a>, index: &apwm::Index) -> Result<RoomSettingsBuilder<'a>> {
        let default_manifest = Manifest::from_index_with_latest_versions(index)?;

        Ok(Self {
            base,
            manifest_builder: ManifestFormBuilder::new(index.clone(), default_manifest),
            room_id: None,
            room: RoomSettings::default(index)?,
        })
    }
}
