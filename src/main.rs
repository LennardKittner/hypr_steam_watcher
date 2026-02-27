use std::{ffi::OsString, thread, time::Duration};

use hyprland::{
    data::Clients,
    dispatch::{Dispatch, DispatchType, TagAction, WindowIdentifier},
    event_listener::{EventListener, WindowOpenEvent},
    shared::HyprData,
};
use procfs::process::Process;

fn main() {
    let mut listener = EventListener::new();
    listener.add_window_opened_handler(move |event| {
        while let Err(e) = find_and_tag_steam_game(&event) {
            eprintln!("Error: {e}");
            eprintln!("Trying again");
            // Wait a little before trying again
            thread::sleep(Duration::from_secs(1));
        }
    });
    listener.start_listener().unwrap();
}

fn find_and_tag_steam_game(event: &WindowOpenEvent) -> hyprland::Result<()> {
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
                && let Some(app_id) = env.get(&app_id_key)
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
                    &format!("steam_app_id_{}", app_id.to_str().unwrap_or("0")),
                    Some(WindowIdentifier::Address(address)),
                ))?;
            }
        } else {
            // If Process::new fails the process is either dead or we don't have access right in
            // either case trying again wont fix the error
            return Ok(());
        }
    }
    Ok(())
}
