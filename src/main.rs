use std::{
    collections::HashMap,
    ffi::OsString,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use hyprland::{
    data::Clients,
    dispatch::{Dispatch, DispatchType, TagAction, WindowIdentifier},
    error::HyprError,
    event_listener::EventListener,
    shared::{Address, HyprData},
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
                .conflicts_with_all(["open-callback", "close-callback"])
        )
        .arg(
            Arg::new("callback-arguments")
                .required(false)
                .index(2)
                .conflicts_with_all(["open-callback", "close-callback"])
                .num_args(1..)
                .help("Arguments for the callback. The PID and Steam app ID will be appended to the arguments.")
                .value_terminator(";")
                .allow_hyphen_values(true)
                .value_parser(clap::value_parser!(String))
        )
        .arg(
            Arg::new("open-callback")
                .long("open-callback") 
                .num_args(1..)
                .help("A callback that will be called when a new window of a steam game appears. The PID and Steam app ID will be appended to the arguments.")
                .value_terminator(";")
                .allow_hyphen_values(true)
                .value_parser(clap::value_parser!(String))
                .conflicts_with_all(["callback", "callback-arguments"])
        )
        .arg(
            Arg::new("close-callback")
                .long("close-callback") 
                .num_args(1..)
                .help("A callback that will be called when a new window of a steam game closes. The PID and Steam app ID will be appended to the arguments.")
                .value_terminator(";")
                .allow_hyphen_values(true)
                .value_parser(clap::value_parser!(String))
        )
        .get_matches();

    let tracked_games: Arc<Mutex<HashMap<Address, GameInfo>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let tracked_games_clone = tracked_games.clone();

    let explicit_open_callback = matches
        .get_many::<String>("open-callback")
        .map(|c| c.cloned().collect::<Vec<String>>());

    let implicit_callback = matches.get_one::<String>("callback").cloned();

    let (callback, callback_args) = match (explicit_open_callback, implicit_callback) {
        (Some(c), None) => (Some(c[0].clone()), c[1..].to_vec()),
        (None, Some(c)) => {
            let callback_args: Vec<String> = matches
                .get_many::<String>("callback-arguments")
                .unwrap_or_default()
                .cloned()
                .collect();
            (Some(c), callback_args)
        }
        (None, None) | (Some(_), Some(_)) => (None, Vec::new()),
    };

    let (close_callback, close_callback_args) = if let Some(vals) = matches
        .get_many::<String>("close-callback")
        .map(|c| c.cloned().collect::<Vec<String>>())
    {
        (Some(vals[0].clone()), vals[1..].to_vec())
    } else {
        (None, Vec::new())
    };

    if let Some(callback) = &callback {
        let mut new_args = callback_args.clone();
        new_args.push("<pid>".to_string());
        new_args.push("<steam_app_id>".to_string());
        println!("On game launch calling: {callback} {}", new_args.join(" "));
    }
    if let Some(close_callback) = &close_callback {
        let mut new_args = close_callback_args.clone();
        new_args.push("<pid>".to_string());
        new_args.push("<steam_app_id>".to_string());
        println!(
            "On game close calling: {close_callback} {}",
            new_args.join(" ")
        );
    }

    let mut listener = EventListener::new();
    listener.add_window_opened_handler(move |event| {
        while match find_and_tag_steam_game(event.window_address.clone()) {
            Err(e) => {
                eprintln!("Error: {e}");
                eprintln!("Trying again");
                // Wait a little before trying again
                thread::sleep(Duration::from_secs(1));
                true
            }
            Ok(GameResult::Game(game)) => {
                if let Some(script_path) = &callback {
                    let mut new_args = callback_args.clone();
                    new_args.push(game.pid.to_string());
                    new_args.push(game.app_id.clone());
                    let mut child = Command::new(script_path).args(&new_args).spawn().unwrap();
                    // call wait to avoid zombies but do not block
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                tracked_games
                    .lock()
                    .unwrap()
                    .insert(event.window_address.clone(), game);
                false
            }
            Ok(GameResult::NoSteamGame) | Ok(GameResult::ProcessNotFound) => false,
        } {}
    });

    listener.add_window_closed_handler(move |address| {
        if let Some(game) = tracked_games_clone.lock().unwrap().remove(&address)
            && let Some(script_path) = &close_callback
        {
            let mut new_args = close_callback_args.clone();
            new_args.push(game.pid.to_string());
            new_args.push(game.app_id.clone());
            let mut child = Command::new(script_path).args(&new_args).spawn().unwrap();
            // call wait to avoid zombies but do not block
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    });
    listener.start_listener().unwrap();
}

#[derive(Debug)]
struct GameInfo {
    pid: i32,
    app_id: String,
}

#[derive(Debug)]
enum GameResult {
    Game(GameInfo),
    ProcessNotFound,
    NoSteamGame,
}

fn find_and_tag_steam_game(address: Address) -> Result<GameResult, HyprError> {
    let app_id_key = OsString::from("SteamAppId");
    let clients = Clients::get()?;
    if let Some((pid, address)) = clients.iter().find_map(|c| {
        if c.address == address {
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
                return Ok(GameResult::Game(GameInfo { pid, app_id }));
            }
        } else {
            // If Process::new fails the process is either dead or we don't have access right in
            // either case trying again wont fix the error
            return Ok(GameResult::ProcessNotFound);
        }
    }
    Ok(GameResult::NoSteamGame)
}
