use std::path::{Path, PathBuf};
use anyhow::Result;

use apwm::Index;
use git2::{Repository, ResetType};

pub struct IndexManager {
    index: Index,
    index_path: PathBuf,
    index_repo_url: String,
    pub apworlds_path: PathBuf,
}

impl IndexManager {
    pub fn new() -> Result<Self> {
        let index_repo_url = std::env::var("APWORLDS_INDEX_REPO_URL").expect("Provide a `APWORLDS_INDEX_REPO_URL` env variable");

        let index_path = std::path::PathBuf::from(
            std::env::var("APWORLDS_INDEX_DIR").unwrap_or_else(|_| "./index".into())
        );

        clone_or_update(&index_repo_url, &index_path)?;

        let index_file = index_path.join("index.toml");
        let index = apwm::Index::new(&index_file)?;

        let apworlds_path = std::path::PathBuf::from(
            std::env::var("APWORLDS_PATH").expect("Provide a `APWORLDS_PATH` env variable"),
        );

        let manager = Self {
            index,
            apworlds_path,
            index_path,
            index_repo_url
        };

        Ok(manager)
    }

    pub async fn update(&mut self) -> Result<()> {
        clone_or_update(&self.index_repo_url, &self.index_path)?;
        let new_index = self.parse_index()?;
        new_index.refresh_into(&self.apworlds_path, false).await?;
        self.index = new_index;


        Ok(())
    }

    fn parse_index(&self) -> Result<Index> {
        let index_file = self.index_path.join("index.toml");
        let index = apwm::Index::new(&index_file)?;

        Ok(index)
    }

    pub fn index(&self) -> &Index {
        &self.index
    }
}

fn clone_or_update(repo_url: &str, path: &Path) -> Result<()> {
    let repo = Repository::init(path)?;

    let mut remote = repo.find_remote("origin").or_else(|_|repo.remote("origin", repo_url))?;

    remote.fetch(&["main"], None, None)?;
    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    repo.reset(&fetch_head.peel(git2::ObjectType::Commit)?, ResetType::Hard, None)?;

    Ok(())
}
