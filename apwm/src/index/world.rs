use crate::{
    utils::{de, git_clone_shallow},
    IndexLock, VersionReq,
};
use anyhow::{bail, Context, Result};
use http::Uri;
use reqwest::{Client, Url};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tempfile::{tempdir, TempDir};

#[derive(Deserialize, Debug, PartialEq, Default, Clone)]
pub enum WorldOrigin {
    #[serde(rename = "url")]
    Url(#[serde(with = "http_serde::uri")] Uri),
    #[serde(rename = "local")]
    Local(PathBuf),
    Supported,
    #[default]
    Default,
}

impl WorldOrigin {
    pub fn is_local(&self) -> bool {
        matches!(self, WorldOrigin::Local(_))
    }

    pub fn is_supported(&self) -> bool {
        matches!(self, WorldOrigin::Supported)
    }

    // TODO: Add support for patching
    pub fn has_patches(&self) -> bool {
        false
    }
}

static AP_CACHE: OnceLock<TempDir> = OnceLock::new();

#[derive(Deserialize, Debug, Clone)]
pub struct World {
    #[serde(skip)]
    pub path: PathBuf,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(with = "http_serde::option::uri", default)]
    pub default_url: Option<Uri>,
    #[serde(deserialize_with = "de::version_req_external", default)]
    pub default_version: VersionReq,
    #[serde(deserialize_with = "de::empty_string_as_none", default)]
    pub home: Option<String>,
    #[serde(deserialize_with = "de::map_with_default_value", default)]
    pub versions: BTreeMap<Version, WorldOrigin>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub supported: bool,
}

impl World {
    pub fn new(world_path: &Path) -> Result<Self> {
        let world_content = std::fs::read_to_string(world_path)?;
        let deser = toml::Deserializer::new(&world_content);
        let mut world: Self = serde_path_to_error::deserialize(deser)?;
        world.path = world_path.into();
        if world.display_name.is_empty() {
            world.display_name = world.name.clone();
        }
        Ok(world)
    }

    pub fn get_latest_release(&self) -> Option<(&Version, &WorldOrigin)> {
        self.versions.iter().max_by_key(|p| p.0)
    }

    pub fn get_latest_supported_release(&self) -> Option<(&Version, &WorldOrigin)> {
        self.versions
            .iter()
            .find(|(_, origin)| origin.is_supported())
    }

    pub fn get_version(&self, version: &Version) -> Option<&WorldOrigin> {
        self.versions.get(version)
    }

    pub async fn copy_to(
        &self,
        version: &Version,
        mut destination: &File,
        expected_checksum: Option<String>,
    ) -> Result<String> {
        let url = self.get_url_for_version(version)?;

        let origin = self.versions.get(version).with_context(|| {
            format!("Unable to find version {} for world {}", self.name, version)
        })?;
        match origin {
            WorldOrigin::Default | WorldOrigin::Url(_) => {
                self.download_to(&url, destination, expected_checksum).await
            }
            WorldOrigin::Local(_) => {
                let full_path = self.get_path_for_origin(origin)?;
                let mut src = std::fs::File::open(&full_path)?;
                let mut buf = Vec::new();
                src.read_to_end(&mut buf)?;
                let checksum = format!("{:x}", Sha256::digest(&buf));
                if expected_checksum.is_some() && Some(&checksum) != expected_checksum.as_ref() {
                    bail!("Error while copying apworld {:?}. Checksum didn't match what was expected.", full_path);
                }

                destination.write_all(&buf[..])?;

                Ok(checksum)
            }
            WorldOrigin::Supported => Ok("none".into()),
        }
    }

    pub async fn extract_to(
        &self,
        version: &Version,
        destination: &Path,
        ap_index_url: &str,
        ap_index_ref: &str,
        lock_file: &IndexLock,
        lobby_url: &Option<Url>,
    ) -> Result<()> {
        let origin = self.versions.get(version).with_context(|| {
            format!("Unable to find version {} for world {}", version, self.name)
        })?;

        if origin.is_supported() {
            let ap_cache = AP_CACHE.get_or_init(|| {
                let cache = tempdir().unwrap();
                git_clone_shallow(ap_index_url, ap_index_ref, cache.path()).unwrap();
                cache
            });

            crate::utils::copy_dir_all(
                &ap_cache.path().join("worlds").join(self.get_ap_name()?),
                &destination.join(self.get_ap_name()?),
            )?;

            return Ok(());
        }

        let download_dir = tempdir()?;
        let apworld_path = download_dir.path().join("apworld");
        let apworld_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&apworld_path)?;

        let apworld_name = self.get_ap_name()?;
        let expected_checksum = lock_file.get_checksum(&apworld_name, version);
        let checksum = self
            .copy_to(version, &apworld_file, expected_checksum)
            .await;
        if checksum.is_err() {
            if let Some(lobby_url) = lobby_url {
                apworld_file.set_len(0)?;
                self.download_from_lobby(lobby_url, version, &apworld_file)
                    .await?;
            } else {
                bail!(
                    "Couldn't get world for `{}`, version `{}`. {}",
                    apworld_name,
                    version,
                    checksum.unwrap_err()
                );
            }
        }

        let mut archive = zip::ZipArchive::new(File::open(apworld_path)?)?;
        Ok(archive.extract(destination)?)
    }

    pub fn get_ap_name(&self) -> Result<String> {
        Ok(self
            .path
            .file_stem()
            .context("Invalid path for world")?
            .to_string_lossy()
            .to_string())
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
            WorldOrigin::Supported => Ok("https://archipelago.gg/games".into()),
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

    async fn download_to(
        &self,
        uri: &str,
        mut destination: &File,
        expected_checksum: Option<String>,
    ) -> Result<String> {
        let req = reqwest::get(uri).await?.error_for_status()?;
        let body = req.bytes().await?;
        let checksum = format!("{:x}", Sha256::digest(&body));
        if expected_checksum.is_some() && Some(&checksum) != expected_checksum.as_ref() {
            bail!(
                "Error while downloading apworld from {}. Checksum didn't match what was expected.",
                uri
            );
        }

        destination.write_all(&body)?;
        Ok(checksum)
    }

    async fn download_from_lobby(
        &self,
        lobby_url: &Url,
        version: &Version,
        mut destination: &File,
    ) -> Result<String> {
        let api_key = std::env::var("LOBBY_API_KEY")?;
        let client = Client::new();
        let url = format!(
            "{}/worlds/download_cached/{}/{}",
            lobby_url,
            self.get_ap_name()?,
            version
        );
        let req = client
            .get(url.as_str())
            .header("X-Api-Key", api_key)
            .send()
            .await?;
        let body = req.bytes().await?;

        destination.write_all(&body)?;
        let checksum = Sha256::digest(&body);
        Ok(format!("{:x}", checksum))
    }
}
