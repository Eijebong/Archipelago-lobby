use anyhow::{bail, Context, Result};
use http::header::CONTENT_DISPOSITION;
use rocket::http::Header;
use semver::Version;
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio::sync::RwLock;

use apwm::{Index, Manifest};
use git2::{Repository, ResetType};

use crate::utils::ZipFile;

pub struct IndexManager {
    pub index: RwLock<Index>,
    index_path: PathBuf,
    index_repo_url: String,
    index_repo_branch: String,
    pub apworlds_path: PathBuf,
}

impl IndexManager {
    pub fn new() -> Result<Self> {
        let index_repo_url = std::env::var("APWORLDS_INDEX_REPO_URL")
            .expect("Provide a `APWORLDS_INDEX_REPO_URL` env variable");

        let index_repo_branch = std::env::var("APWORLDS_INDEX_REPO_BRANCH")
            .expect("Provide a `APWORLDS_INDEX_REPO_BRANCH` env variable");

        let index_path = std::path::PathBuf::from(
            std::env::var("APWORLDS_INDEX_DIR").unwrap_or_else(|_| "./index".into()),
        );

        clone_or_update(&index_repo_url, &index_repo_branch, &index_path)?;

        let index_file = index_path.join("index.toml");
        let index = apwm::Index::new(&index_file)?;

        let apworlds_path = std::path::PathBuf::from(
            std::env::var("APWORLDS_PATH").expect("Provide a `APWORLDS_PATH` env variable"),
        );

        let manager = Self {
            index: RwLock::new(index),
            apworlds_path,
            index_path,
            index_repo_url,
            index_repo_branch,
        };

        Ok(manager)
    }

    pub async fn update(&self) -> Result<()> {
        clone_or_update(
            &self.index_repo_url,
            &self.index_repo_branch,
            &self.index_path,
        )?;
        let new_index = self.parse_index()?;
        new_index
            .refresh_into(&self.apworlds_path, false, None)
            .await?;
        *self.index.write().await = new_index;

        Ok(())
    }

    fn parse_index(&self) -> Result<Index> {
        let index_file = self.index_path.join("index.toml");
        let index = apwm::Index::new(&index_file)?;

        Ok(index)
    }

    pub async fn get_apworld_from_game_name(
        &self,
        manifest: &Manifest,
        game_name: &str,
    ) -> Option<(String, Version)> {
        let index = self.index.read().await;
        let (world, version) = manifest.resolve_from_game_name(game_name, &index).ok()?;
        let path = world.path.file_stem().unwrap().to_str().unwrap().to_owned();

        Some((path, version.clone()))
    }

    pub async fn download_apworlds(&self, manifest: &Manifest) -> Result<ZipFile<'_>> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let apworlds_path = &self.apworlds_path;
        let prefix = "custom_worlds";
        writer.add_directory(prefix, options)?;

        let index = self.index.read().await;
        let mut buffer = Vec::new();
        let (worlds, resolve_errors) = manifest.resolve_with(&index);
        if !resolve_errors.is_empty() {
            bail!("Error while resolving manifest");
        }

        for (world_name, (world, version)) in &worlds {
            let origin = world.get_version(version).unwrap();

            if origin.is_supported() {
                continue;
            }

            let file_path = index.get_world_local_path(apworlds_path, world_name, version);
            writer.start_file(format!("{prefix}/{world_name}.apworld"), options)?;
            File::open(&file_path)
                .with_context(|| format!("Can't open {file_path:?}"))?
                .read_to_end(&mut buffer)?;
            writer.write_all(&buffer)?;
            buffer.clear();
        }

        let value = "attachment; filename=\"apworlds.zip\"";
        let content = writer.finish()?.into_inner();

        Ok(ZipFile {
            content,
            headers: Header::new(CONTENT_DISPOSITION.as_str(), value),
        })
    }
}

fn clone_or_update(repo_url: &str, repo_branch: &str, path: &Path) -> Result<()> {
    let repo = Repository::init(path)?;

    let mut remote = repo
        .find_remote("origin")
        .or_else(|_| repo.remote("origin", repo_url))?;

    remote.fetch(&[repo_branch], None, None)?;
    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    repo.reset(
        &fetch_head.peel(git2::ObjectType::Commit)?,
        ResetType::Hard,
        None,
    )?;

    Ok(())
}
