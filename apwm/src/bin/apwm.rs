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
}

#[derive(clap::Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[tokio::main]
async fn main() -> Result<()> {
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
