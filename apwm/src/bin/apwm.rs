use anyhow::Result;
use clap::Parser;
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
    },
    Install {
        #[clap(short)]
        index_path: PathBuf,
        #[clap(short)]
        apworlds_path: PathBuf,
        #[clap(short)]
        destination: PathBuf,
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
        } => {
            download(&index_path, &destination).await?;
        }
        Command::Install {
            index_path,
            apworlds_path,
            destination,
        } => {
            install(&index_path, &apworlds_path, &destination).await?;
        }
    }

    Ok(())
}

async fn download(index_path: &Path, destination: &Path) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;
    index.refresh_into(destination, false).await?;

    Ok(())
}

async fn update(index_path: &Path) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;
    let destination = tempdir()?;

    let new_lock = index.refresh_into(destination.path(), true).await?;

    new_lock.write()?;

    Ok(())
}

async fn install(index_path: &Path, apworlds_path: &Path, destination: &Path) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;

    std::fs::create_dir_all(destination)?;

    for (world_name, world) in &index.worlds {
        let Some((version, _)) = world.get_latest_release() else {
            continue;
        };
        let apworld_path = index.get_world_local_path(apworlds_path, world_name, version);
        let destination = destination.join(format!("{}.apworld", world_name));
        std::fs::copy(apworld_path, destination)?;
    }

    Ok(())
}
