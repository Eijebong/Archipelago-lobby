use anyhow::{bail, Context, Result};
use apwm::diff::diff_world_and_write;
use apwm::utils::git_clone_shallow;
use clap::Parser;
use reqwest::Url;
use semver::Version;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[derive(clap::Subcommand)]
enum Command {
    Update {
        #[clap(short)]
        index_path: PathBuf,
    },
    Download {
        #[clap(short)]
        index_path: PathBuf,
        #[clap(short)]
        destination: PathBuf,
        #[clap(short)]
        precise: Option<String>,
    },
    Install {
        #[clap(short)]
        index_path: PathBuf,
        #[clap(short)]
        apworlds_path: PathBuf,
        #[clap(short)]
        destination: PathBuf,
        #[clap(short)]
        precise: Option<String>,
    },
    Diff {
        #[clap(short)]
        index_path: PathBuf,
        #[clap(short)]
        from: String,
        #[clap(short = 'r')]
        from_ref: Option<String>,
        #[clap(short)]
        output: PathBuf,
        #[clap(short)]
        lobby_url: Option<Url>,
    },
}

#[derive(clap::Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Args::parse();
    match cli.command {
        Command::Update { index_path } => {
            update(&index_path).await?;
        }
        Command::Download {
            index_path,
            destination,
            precise,
        } => {
            download(&index_path, &destination, &precise).await?;
        }
        Command::Install {
            index_path,
            apworlds_path,
            destination,
            precise,
        } => {
            install(&index_path, &apworlds_path, &destination, &precise).await?;
        }
        Command::Diff {
            index_path,
            from,
            from_ref,
            output,
            lobby_url,
        } => {
            if lobby_url.is_some() {
                if std::env::var("LOBBY_API_KEY").is_err() {
                    bail!("Lobby url specified but missing `LOBBY_API_KEY` env variable");
                }
            }
            diff(&index_path, &from, &from_ref, &output, &lobby_url).await?;
        }
    }

    Ok(())
}

async fn download(index_path: &Path, destination: &Path, precise: &Option<String>) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;
    let target = apworld_version_from_precise(precise)?;

    index.refresh_into(destination, false, target).await?;

    Ok(())
}

async fn update(index_path: &Path) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;
    let destination = tempdir()?;

    let new_lock = index.refresh_into(destination.path(), true, None).await?;

    new_lock.write()?;

    Ok(())
}

async fn diff(
    index_path: &Path,
    from_git_remote: &str,
    from_git_ref: &Option<String>,
    output: &Path,
    lobby_url: &Option<Url>,
) -> Result<()> {
    let old_index_dir = tempdir()?;
    git_clone_shallow(
        from_git_remote,
        from_git_ref.as_ref().map_or("main", |v| v),
        old_index_dir.path(),
    )?;

    let new_index_toml = index_path.join("index.toml");
    let old_index_toml = old_index_dir.path().join("index.toml");
    let old_index_lock = old_index_dir.path().join("index.lock");

    let new_index = apwm::Index::new(&new_index_toml)?;
    let old_index = apwm::Index::new(&old_index_toml)?;
    let old_index_lock = apwm::IndexLock::new(&old_index_lock)?;

    let old_worlds = old_index.worlds;
    let new_worlds = new_index.worlds;

    for (name, world) in &new_worlds {
        match old_worlds.get(name) {
            // This is a new world, diff from nothing
            None => {
                diff_world_and_write(
                    None,
                    Some(world),
                    name,
                    output,
                    &new_index.archipelago_repo.to_string(),
                    &new_index.archipelago_version.to_string(),
                    &old_index_lock,
                    lobby_url,
                )
                .await?
            }
            // The world was already there before, diff from latest version
            Some(old_world) => {
                if world.versions.keys().collect::<Vec<_>>()
                    == old_world.versions.keys().collect::<Vec<_>>()
                {
                    continue;
                }
                diff_world_and_write(
                    Some(old_world),
                    Some(world),
                    name,
                    output,
                    &new_index.archipelago_repo.to_string(),
                    &new_index.archipelago_version.to_string(),
                    &old_index_lock,
                    lobby_url,
                )
                .await?
            }
        }
    }

    for (name, world) in &old_worlds {
        if !new_worlds.contains_key(name.as_str()) {
            diff_world_and_write(
                Some(world),
                None,
                name,
                output,
                &new_index.archipelago_repo.to_string(),
                &new_index.archipelago_version.to_string(),
                &old_index_lock,
                lobby_url,
            )
            .await?;
        }
    }

    Ok(())
}

fn apworld_version_from_precise(precise: &Option<String>) -> Result<Option<(String, Version)>> {
    if let Some(precise) = precise {
        let parts = precise.splitn(2, ':').collect::<Vec<_>>();
        if parts.len() != 2 {
            anyhow::bail!("Precise version need to be of the form <apworld>:<version>");
        }

        Ok(Some((parts[0].to_string(), parts[1].parse::<Version>()?)))
    } else {
        Ok(None)
    }
}

async fn install(
    index_path: &Path,
    apworlds_path: &Path,
    destination: &Path,
    precise: &Option<String>,
) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;

    std::fs::create_dir_all(destination).context("While creating destination dir")?;
    let target = apworld_version_from_precise(precise)?;

    for (world_name, world) in &index.worlds {
        let version = if let Some((ref target_apworld, ref target_version)) = target {
            if target_apworld != world_name {
                continue;
            }
            target_version
        } else {
            let Some((version, _)) = world.get_latest_release() else {
                continue;
            };
            version
        };

        let apworld_path = index.get_world_local_path(apworlds_path, world_name, version);
        let destination = destination.join(format!("{}.apworld", world_name));
        std::fs::copy(&apworld_path, &destination)
            .with_context(|| format!("Cannot copy {:?} to {:?}", &apworld_path, &destination))?;
    }

    Ok(())
}
