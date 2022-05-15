use slickscreen::{Slickscreen, SlickscreenConfig};

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use ctrlc;
use std::sync::mpsc::channel;

use scrap::Display;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
/// A tool for capturing your screen and system audio
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    FileCapture(FileCaptureArguments),
    ListScreens,
}

#[derive(Args, Debug)]
/// Record the screen capture to a file
///
/// The default is VP8 and Opus in a Vorbis container
struct FileCaptureArguments {
    #[clap(long, short = 'o')]
    output_file: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let (ctrlc_tx, ctrlc_rx) = channel();
    ctrlc::set_handler(move || {
        ctrlc_tx
            .send(())
            .expect("Could not send ctrl+c signal to application.")
    })?;

    match &cli.command {
        Commands::FileCapture(args) => {
            let config = SlickscreenConfig {
                output_file: Some(args.output_file.clone()),
                ..SlickscreenConfig::default()
            };
            let slick = Slickscreen::new(config)?;

            ctrlc_rx.recv()?;

            println!("Stopping Slickscreen... ");
            slick.stop();
            println!("Slickscreen stopped - Exiting.");
        }
        Commands::ListScreens => {
            if let Ok(displays) = Display::all() {
                for (i, display) in displays.iter().enumerate() {
                    println!("{}: {}x{}", i, display.width(), display.height());
                }
            } else {
                println!("No displays were found");
            }
        }
    }

    Ok(())
}
