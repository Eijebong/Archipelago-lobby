use crate::utils::de;
use anyhow::{bail, Context, Result};
use http::Uri;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Deserialize, Debug, PartialEq, Default, Clone)]
pub enum WorldOrigin {
    #[serde(rename = "url")]
    Url(#[serde(with = "http_serde::uri")] Uri),
    #[serde(rename = "local")]
    Local(PathBuf),
    #[default]
    Default,
}

impl WorldOrigin {
    pub fn is_local(&self) -> bool {
        matches!(self, WorldOrigin::Local(_))
    }

    // TODO: Add support for patching
    pub fn has_patches(&self) -> bool {
        false
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct World {
    #[serde(skip)]
    pub path: PathBuf,
    pub name: String,
    #[serde(with = "http_serde::option::uri", default)]
    pub default_url: Option<Uri>,
    #[serde(deserialize_with = "de::empty_string_as_none", default)]
    pub home: Option<String>,
    #[serde(deserialize_with = "de::map_with_default_value", default)]
    pub versions: BTreeMap<Version, WorldOrigin>,
    #[serde(default)]
    pub disabled: bool,
}

impl World {
    pub fn new(world_path: &Path) -> Result<Self> {
        let world_content = std::fs::read_to_string(world_path)?;
        let deser = toml::Deserializer::new(&world_content);
        let mut world: Self = serde_path_to_error::deserialize(deser)?;
        world.path = world_path.into();

        Ok(world)
    }

    pub fn get_latest_release(&self) -> Option<(&Version, &WorldOrigin)> {
        self.versions.iter().max_by_key(|p| p.0)
    }

    pub fn get_version(&self, version: &Version) -> Option<&WorldOrigin> {
        self.versions.get(version)
    }

    pub async fn copy_to(&self, version: &Version, mut destination: &File) -> Result<String> {
        let url = self.get_url_for_version(version)?;

        let origin = self.versions.get(version).with_context(|| {
            format!("Unable to find version {} for world {}", self.name, version)
        })?;
        match origin {
            WorldOrigin::Default | WorldOrigin::Url(_) => self.download_to(&url, destination).await,
            WorldOrigin::Local(_) => {
                let full_path = self.get_path_for_origin(origin)?;
                let mut src = std::fs::File::open(full_path)?;
                let mut buf = Vec::new();
                src.read_to_end(&mut buf)?;

                destination.write_all(&buf[..])?;

                let checksum = Sha256::digest(&buf);
                Ok(format!("{:x}", checksum))
            }
        }
    }

    pub fn get_url_for_version(&self, version: &Version) -> Result<String> {
        let origin = self.versions.get(version).with_context(|| {
            format!("Unable to find version {} for world {}", self.name, version)
        })?;

        match origin {
            WorldOrigin::Default => {
                let url = self.default_url.as_ref().with_context(|| {
                    format!(
                        "World {} has no default URL but contains a release ({}) without a set URL",
                        self.name, version
                    )
                })?;
                let url = url.to_string().replace("{{version}}", &version.to_string());
                Ok(url)
            }
            WorldOrigin::Url(url) => {
                let url = url.to_string().replace("{{version}}", &version.to_string());
                Ok(url)
            }
            WorldOrigin::Local(_) => Ok("".into()),
        }
    }

    pub fn get_path_for_origin(&self, origin: &WorldOrigin) -> Result<PathBuf> {
        match origin {
            WorldOrigin::Local(path) => {
                let full_path = self.path.parent().context("Invalid world path")?.join(path);
                Ok(full_path)
            }
            _ => bail!("This isn't a local world origin, no path"),
        }
    }

    async fn download_to(&self, uri: &str, mut destination: &File) -> Result<String> {
        let req = reqwest::get(uri).await?;
        let body = req.bytes().await?;

        destination.write_all(&body)?;
        let checksum = Sha256::digest(&body);
        Ok(format!("{:x}", checksum))
    }
}
