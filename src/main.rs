use std::{ffi::OsString, process::Command, thread, time::Duration};

use hyprland::{
    data::Clients,
    dispatch::{Dispatch, DispatchType, TagAction, WindowIdentifier},
    error::HyprError,
    event_listener::{EventListener, WindowOpenEvent},
    shared::HyprData,
};

use clap::Arg;
use clap::Command as ClapCommand;
use procfs::process::Process;
use std::env;

fn main() {
    let matches = ClapCommand::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about("Automatically tag newly launched Steam games in Hyprland.")
        .arg(
            Arg::new("callback")
                .required(false)
                .help("A callback that will be called when a new window of a steam game appears.")
                .index(1)
        )
        .arg(
            Arg::new("callback-arguments")
                .required(false)
                .index(2)
                .num_args(1..)
                .help("Arguments for the callback. The PID and Steam app ID will be appended to the arguments.")
                .allow_hyphen_values(true)
                .value_parser(clap::value_parser!(String)),
        ).get_matches();

    let callback = matches.get_one::<String>("callback").cloned();
    let callback_args: Vec<String> = matches
        .get_many::<String>("callback-arguments")
        .unwrap_or_default()
        .cloned()
        .collect();

    let mut listener = EventListener::new();
    listener.add_window_opened_handler(move |event| {
        while match find_and_tag_steam_game(&event) {
            Err(e) => {
                eprintln!("Error: {e}");
                eprintln!("Trying again");
                // Wait a little before trying again
                thread::sleep(Duration::from_secs(1));
                true
            }
            Ok(GameResult::Game { pid, app_id }) => {
                if let Some(script_path) = &callback {
                    let mut new_args = callback_args.clone();
                    new_args.push(pid.to_string());
                    new_args.push(app_id);
                    let mut child = Command::new(script_path).args(&new_args).spawn().unwrap();
                    // call wait to avoid zombies but do not block
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                false
            }
            Ok(GameResult::NoSteamGame) | Ok(GameResult::ProcessNotFound) => false,
        } {}
    });
    listener.start_listener().unwrap();
}

enum GameResult {
    Game { pid: i32, app_id: String },
    ProcessNotFound,
    NoSteamGame,
}

fn find_and_tag_steam_game(event: &WindowOpenEvent) -> Result<GameResult, HyprError> {
    let app_id_key = OsString::from("SteamAppId");
    let clients = Clients::get()?;
    if let Some((pid, address)) = clients.iter().find_map(|c| {
        if c.address == event.window_address {
            Some((c.pid, c.address.clone()))
        } else {
            None
        }
    }) {
        if let Ok(process) = Process::new(pid) {
            if let Ok(env) = process.environ()
                && let Some(app_id) = env.get(&app_id_key).cloned()
                && let Ok(app_id) = app_id.into_string()
            {
                // We assume that ever process which has SteamAppId set in its environment is a
                // steam game
                Dispatch::call(DispatchType::TagWindow(
                    TagAction::Add,
                    "steam_game",
                    Some(WindowIdentifier::Address(address.clone())),
                ))?;
                Dispatch::call(DispatchType::TagWindow(
                    TagAction::Add,
                    &format!("steam_app_id_{}", app_id),
                    Some(WindowIdentifier::Address(address)),
                ))?;
                return Ok(GameResult::Game { pid, app_id });
            }
        } else {
            // If Process::new fails the process is either dead or we don't have access right in
            // either case trying again wont fix the error
            return Ok(GameResult::ProcessNotFound);
        }
    }
    Ok(GameResult::NoSteamGame)
}
