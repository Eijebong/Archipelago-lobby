use std::fmt::Display;

use ap_lobby::db::{Room, RoomSettings, RoomTemplate};
use ap_lobby::error::Result;
use apwm::Manifest;
use askama::Template;
use uuid::Uuid;

use crate::TplContext;

use super::manifest_editor::ManifestFormBuilder;

pub enum RoomSettingsType {
    Room,
    Template,
}

impl RoomSettingsType {
    pub fn as_route_base(&self) -> &str {
        match self {
            RoomSettingsType::Room => "/edit-room",
            RoomSettingsType::Template => "/room-templates",
        }
    }

    pub fn is_room(&self) -> bool {
        matches!(self, RoomSettingsType::Room)
    }
}

impl Display for RoomSettingsType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoomSettingsType::Room => f.write_str("room"),
            RoomSettingsType::Template => f.write_str("template"),
        }
    }
}

#[derive(Template)]
#[template(path = "room_manager/room_form.html")]
pub struct RoomSettingsBuilder<'a> {
    base: TplContext<'a>,
    room: RoomSettings,
    manifest_builder: ManifestFormBuilder,
    room_id: Option<Uuid>,
    ty: RoomSettingsType,
    read_only: bool,
    tpl: Option<RoomTemplateBuilder>,
}

pub struct RoomTemplateBuilder {
    pub name: String,
    pub global: bool,
}

impl RoomTemplateBuilder {
    fn from_template(tpl: &RoomTemplate) -> Self {
        Self {
            name: tpl.tpl_name.clone(),
            global: tpl.global,
        }
    }

    fn new() -> Self {
        Self {
            name: "".to_string(),
            global: false,
        }
    }
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
            room_id: Some(room.id.as_generic_id()),
            ty: RoomSettingsType::Room,
            read_only: false,
            tpl: None,
        }
    }

    pub fn room_from_template(
        base: TplContext<'a>,
        index: apwm::Index,
        mut tpl: RoomTemplate,
    ) -> Result<RoomSettingsBuilder<'a>> {
        // Override the close date as to not use the one from the template.
        // The template close data is just there because we reuse the room settings struct and
        // actually corresponds to the template creation date.
        //
        // XXX: In the future we might want to use the time part of the tpl close date, set to the
        // current day.
        tpl.settings.close_date = RoomSettings::default_close_date()?;

        Ok(Self {
            base,
            manifest_builder: ManifestFormBuilder::new(index, tpl.settings.manifest.0.clone()),
            room: tpl.settings,
            room_id: None,
            ty: RoomSettingsType::Room,
            read_only: false,
            tpl: None,
        })
    }

    pub fn new_with_template(
        base: TplContext<'a>,
        index: apwm::Index,
        tpl: RoomTemplate,
    ) -> RoomSettingsBuilder<'a> {
        Self {
            base,
            manifest_builder: ManifestFormBuilder::new(index, tpl.settings.manifest.0.clone()),
            tpl: Some(RoomTemplateBuilder::from_template(&tpl)),
            room: tpl.settings,
            room_id: Some(tpl.id.as_generic_id()),
            ty: RoomSettingsType::Template,
            read_only: false,
        }
    }

    pub fn new(
        base: TplContext<'a>,
        index: &apwm::Index,
        ty: RoomSettingsType,
    ) -> Result<RoomSettingsBuilder<'a>> {
        let tpl = if ty.is_room() {
            None
        } else {
            Some(RoomTemplateBuilder::new())
        };

        let default_manifest = Manifest::from_index_with_latest_versions(index)?;

        Ok(Self {
            base,
            manifest_builder: ManifestFormBuilder::new(index.clone(), default_manifest),
            room_id: None,
            room: RoomSettings::default(index)?,
            ty,
            read_only: false,
            tpl,
        })
    }

    pub fn read_only(mut self, ro: bool) -> Self {
        self.read_only = ro;
        self
    }
}
