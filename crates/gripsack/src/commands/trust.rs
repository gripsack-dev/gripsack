//! Remote repos are trusted on first use, by origin URL (#409). This lists and
//! revokes those records; there is no approval to grant here — running grip on
//! a local repo is consent, and a remote asks once when it is first used.
use super::remote;
use crate::render::Palette;
use clap::Subcommand;
use std::process::ExitCode;

#[derive(Debug, Subcommand)]
pub enum TrustCommand {
    /// List remote origins trusted on first use.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Forget a trusted remote; its next use asks again.
    Remove {
        /// The origin URL as given to `--repo`.
        url: String,
    },
}

pub fn trust(command: TrustCommand, _palette: Palette) -> ExitCode {
    let home = gripsack_host::store::gripsack_home();
    match command {
        TrustCommand::List { json } => {
            let remotes = remote::list(&home);
            if json {
                match serde_json::to_string_pretty(&serde_json::json!({ "remotes": remotes })) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        eprintln!("grip: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            } else if remotes.is_empty() {
                println!("no trusted remotes");
            } else {
                for entry in remotes {
                    println!("{}", gripsack_host::process::terminal::tame(entry));
                }
            }
            ExitCode::SUCCESS
        }
        TrustCommand::Remove { url } => match remote::forget(&home, &url) {
            Ok(true) => {
                println!(
                    "forgot {}",
                    gripsack_host::process::terminal::tame(remote::normalize(&url))
                );
                ExitCode::SUCCESS
            }
            Ok(false) => {
                eprintln!("grip: that remote is not trusted");
                ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("grip: {error}");
                ExitCode::FAILURE
            }
        },
    }
}
