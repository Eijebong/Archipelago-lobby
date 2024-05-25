use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(clap::Subcommand)]
enum Command {
    Refresh {
        #[clap(short)]
        index_path: PathBuf,
        #[clap(short = 'd')]
        apworlds_path: PathBuf,
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
        Command::Refresh {
            index_path,
            apworlds_path,
        } => {
            refresh(&index_path, &apworlds_path).await?;
        }
    }

    Ok(())
}

async fn refresh(index_path: &Path, destination: &Path) -> Result<()> {
    let index_toml = index_path.join("index.toml");
    let index = apwm::Index::new(&index_toml)?;

    if !index.should_refresh(&destination) {
        println!("The index hasn't been changed since the last refresh, nothing to do.");
        return Ok(());
    }

    println!("Refreshing apworlds into {}", destination.to_string_lossy());
    index.refresh_into(destination).await?;

    Ok(())
}
