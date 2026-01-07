use std::fmt::Display;
use std::str::FromStr;

use crate::db::{Room, RoomSettings, RoomTemplate};
use crate::error::Result;
use anyhow::Context as _;
use apwm::Manifest;
use askama::Template;
use askama_web::WebTemplate;
use chrono::{DateTime, TimeZone, Utc};
use rocket::http;
use rocket::FromForm;
use uuid::Uuid;

use crate::TplContext;

use super::manifest_editor::{ManifestForm, ManifestFormBuilder};

#[derive(FromForm, Debug)]
pub struct CreateRoomForm<'a> {
    pub room: RoomSettingsForm<'a>,
}

#[derive(FromForm, Debug)]
pub struct RoomSettingsForm<'a> {
    pub room_name: &'a str,
    pub room_description: &'a str,
    pub close_date: &'a str,
    pub tz_offset: i32,
    pub room_url: &'a str,
    pub yaml_validation: bool,
    pub allow_unsupported: bool,
    pub allow_invalid_yamls: bool,
    pub yaml_limit_per_user: bool,
    pub yaml_limit_per_user_nb: i32,
    pub yaml_limit_bypass_list: &'a str,
    pub show_apworlds: bool,
    pub me: ManifestForm<'a>,
    pub meta_file: String,
    pub is_bundle_room: bool,
}

pub fn parse_date(date: &str, tz_offset: i32) -> Result<DateTime<Utc>> {
    let offset = chrono::FixedOffset::west_opt(tz_offset * 60)
        .ok_or_else(|| crate::error::Error(anyhow::anyhow!("Wrong timezone offset")))?;
    let datetime = chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M")?;
    let date = offset
        .from_local_datetime(&datetime)
        .single()
        .ok_or_else(|| crate::error::Error(anyhow::anyhow!("Cannot parse passed datetime")))?;

    Ok(date.into())
}

pub fn validate_room_form(room_form: &mut RoomSettingsForm<'_>) -> Result<()> {
    if room_form.room_name.trim().is_empty() {
        return Err(anyhow::anyhow!("The room name shouldn't be empty").into());
    }

    if room_form.room_name.len() > 200 {
        return Err(anyhow::anyhow!("The room name shouldn't exceed 200 characters. Seriously it doesn't need to be that long.").into());
    }

    let room_url = room_form.room_url.trim();
    if !room_url.is_empty() {
        if let Err(e) = http::uri::Uri::parse::<http::uri::Absolute>(room_url) {
            return Err(anyhow::anyhow!("Error while parsing room URL: {}", e).into());
        }
    }
    room_form.room_url = room_url;

    if room_form.yaml_limit_per_user && room_form.yaml_limit_per_user_nb <= 0 {
        return Err(
            anyhow::anyhow!("The per player YAML limit should be greater or equal to 1").into(),
        );
    }

    if !room_form.yaml_limit_bypass_list.is_empty() {
        let possible_ids = room_form.yaml_limit_bypass_list.split(',');
        for possible_id in possible_ids {
            if i64::from_str(possible_id).is_err() {
                return Err(anyhow::anyhow!(
                    "The YAML limit bypass list should be a comma delimited list of discord IDs."
                )
                .into());
            }
        }
    }

    room_form.meta_file = room_form.meta_file.trim().to_string();
    if !room_form.meta_file.is_empty() {
        serde_saphyr::from_str::<MetaFile>(&room_form.meta_file)
            .context("Failed to parse meta file. Make sure it includes a `meta_description`")?;
    }

    Ok(())
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
pub struct MetaFile {
    meta_description: String,
}

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

#[derive(Template, WebTemplate)]
#[template(path = "shared/room_form.html")]
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
        let new_manifest = tpl.settings.manifest.0.updated_with_index(&index)?;

        Ok(Self {
            base,
            manifest_builder: ManifestFormBuilder::new(index, new_manifest),
            room: tpl.settings.into(),
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
            room: tpl.settings.into(),
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

        let default_manifest = Manifest::from_index_with_default_versions(index)?;

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
