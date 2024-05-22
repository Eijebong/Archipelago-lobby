use std::{collections::BTreeMap, fs::{remove_dir_all, OpenOptions}, path::{Path, PathBuf}};
use anyhow::Result;
use serde::Deserialize;
use http::Uri;
use git2::{build::RepoBuilder, AutotagOption, FetchOptions};



fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(&dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), &dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn delete_file_or_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else if path.is_file() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[derive(Deserialize, Debug)]
pub struct Common {
    #[serde(with = "http_serde::uri")]
    pub archipelago_repo: Uri,
    pub archipelago_version: String,
    pub homepage: String,
    pub required_global_files: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub enum WorldOrigin {
    #[serde(rename = "url")]
    Url(#[serde(with = "http_serde::uri")] Uri),
    #[serde(rename = "supported")]
    Supported(String),
    #[serde(rename = "local")]
    Local(PathBuf),
}

impl WorldOrigin {
    pub fn is_supported(&self) -> bool {
        matches!(self, WorldOrigin::Supported(_))
    }
}

impl World {
    async fn download_to(&self, destination: &Path, ap_dir: &Path, index_dir: &Path) -> Result<()> {
        match &self.origin {
            WorldOrigin::Url(uri) => self.download_uri(uri, destination).await,
            WorldOrigin::Supported(apworld) => self.download_supported(destination, ap_dir, &apworld).await,
            WorldOrigin::Local(path) => self.copy_local(destination, index_dir, &path),
        }
    }

    fn copy_local(&self, destination: &Path, index_dir: &Path, local_path: &Path) -> Result<()> {
        if destination.exists() {
            delete_file_or_dir(destination)?;
        }

        let path = index_dir.join(local_path);
        if path.is_dir() {
            copy_dir_all(&path, &destination)?;
        } else if path.is_file() {
            std::fs::copy(&path, &destination)?;
        }

        Ok(())
    }

    async fn download_uri(&self, uri: &Uri, destination: &Path) -> Result<()> {
        if destination.exists() {
            std::fs::remove_file(destination)?;
        }

        let req = reqwest::get(&uri.to_string()).await?;
        let body = req.bytes().await?;
        std::fs::write(destination, body)?;

        Ok(())
    }

    async fn download_supported(&self, destination: &Path, ap_dir: &Path, dir_name: &str) -> Result<()> {
        let world_destination = destination.join(dir_name);
        if world_destination.exists() {
            std::fs::remove_dir_all(&world_destination)?;
        }

        let apworld_dir = ap_dir.join("worlds").join(dir_name);
        copy_dir_all(&apworld_dir, &world_destination)?;

        for dependency in &self.dependencies {
            let dep_path = ap_dir.join("worlds").join(dependency);
            let dep_destination = destination.join(dependency);

            if dep_destination.exists() {
                std::fs::remove_dir_all(&dep_destination)?;
            }

            if dep_path.is_dir() {
                copy_dir_all(&dep_path, &dep_destination)?;
            } else if dep_path.is_file() {
                std::fs::copy(&dep_path, &dep_destination)?;
            }
        }

        Ok(())
    }

    pub fn version(&self) -> &str {
        self.version.as_ref().map(String::as_str).unwrap_or("Unknown")
    }

    pub fn url(&self) -> String {
        match self.origin {
            WorldOrigin::Url(ref url) => url.to_string(),
            WorldOrigin::Supported(_) | WorldOrigin::Local(_) => "".into(),
        }
    }

    pub fn has_patches(&self) -> bool {
        !self.patches.is_empty()
    }

    pub fn is_supported(&self) -> bool {
        self.origin.is_supported()
    }
}

#[derive(Deserialize, Debug)]
pub struct World {
    pub name: String,
    #[serde(flatten)]
    pub origin: WorldOrigin,
    version: Option<String>,
    #[serde(default)]
    patches: Vec<String>,
    pub home: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}


#[derive(Deserialize, Debug)]
pub struct Index {
    #[serde(skip)]
    path: PathBuf,
    pub common: Common,
    pub worlds: BTreeMap<String, World>,
}

impl Index {
    pub fn new(index_path: &Path) -> Result<Self> {
        let index_content = std::fs::read_to_string(index_path)?;
        let deser = toml::Deserializer::new(&index_content);

        let mut index: Index = serde_path_to_error::deserialize(deser)?;
        index.path = index_path.into();

        for (_, world) in index.worlds.iter_mut(){
            if world.origin.is_supported() {
                world.version = Some(index.common.archipelago_version.clone());
            }
        }

        Ok(index)
    }

    pub async fn refresh_into(&self, destination: &Path) -> Result<()> {
        let ap_tmp_dir = tempfile::tempdir()?;
        let ap_tmp_dir = ap_tmp_dir.path();

        {
            let mut fetch_opts = FetchOptions::new();
            fetch_opts.download_tags(AutotagOption::All);

            let repo = RepoBuilder::new()
                .fetch_options(fetch_opts)
                .clone(&self.common.archipelago_repo.to_string(), &ap_tmp_dir)?;
            let git_ref = repo.resolve_reference_from_short_name(&self.common.archipelago_version)?;
            let tag = git_ref.peel_to_commit()?;

            repo.checkout_tree(&tag.as_object(), None)?;
        }

        if destination.exists() {
            remove_dir_all(destination)?;
        }
        std::fs::create_dir_all(destination)?;

        let index_dir = self.path.parent().ok_or_else(|| anyhow::anyhow!("Index file doesn't have a parent dir"))?;
        for (name, world) in &self.worlds {
            let world_dest = match &world.origin {
                WorldOrigin::Local(path) => destination.join(path.file_name().unwrap()),
                WorldOrigin::Supported(_) => destination.into(),
                WorldOrigin::Url(_) => destination.join(&format!("{}.apworld", name))
            };

            world.download_to(&world_dest, &ap_tmp_dir, &index_dir).await?
        }

        for path in &self.common.required_global_files {
            let file_path = ap_tmp_dir.join("worlds").join(path);
            let file_destination = destination.join(path);

            if file_destination.exists() {
                delete_file_or_dir(&file_destination)?;
            }

            if file_path.is_dir() {
                copy_dir_all(&file_path, &file_destination)?;
            } else if file_path.is_file() {
                std::fs::copy(&file_path, &file_destination)?;
            }
        }

        let last_refreshed = destination.join(".last_refresh");
        OpenOptions::new().create(true).write(true).open(last_refreshed)?;

        Ok(())
    }

    pub fn should_refresh(&self, destination: &Path) -> bool {
        let last_refreshed = destination.join(".last_refresh");

        let Ok(last_refreshed_metadata) = std::fs::metadata(last_refreshed) else { return true };
        let Ok(index_metadata) = std::fs::metadata(&self.path) else { return true };

        let Ok(last_refreshed_mtime) = last_refreshed_metadata.modified() else { return true };
        let Ok(index_mtime) = index_metadata.modified() else { return true };

        index_mtime > last_refreshed_mtime
    }
}

