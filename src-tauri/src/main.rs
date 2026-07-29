// The desktop binary must not pop a console window behind the webview on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! **reeve-app — ring 1, and the composition root.**
//!
//! This file is the single place in the codebase where a concrete `reeve-infra`
//! implementation is named and handed to a `reeve-core` service. Everywhere
//! else, core talks to a trait from `reeve_core::seams` and does not know what
//! is behind it. If wiring appears anywhere but here, the ring rule has leaked.
//!
//! The other two responsibilities of this crate live beside it: [`commands`]
//! (one `#[tauri::command]` per operation in 03-api, each a pure delegation)
//! and [`events`] (the four-event emitter, the closed catalog).

mod commands;
mod events;

fn main() {
    tauri::Builder::default()
        // Composition happens here, in `.setup`: each subsystem ticket constructs
        // its `reeve-infra` implementation, wraps it in the core service that
        // consumes the seam, and puts the result in Tauri's managed state for
        // `commands` to reach. Nothing is wired yet — the floor is bare on purpose.
        .setup(|_app| Ok(()))
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("failed to start the reeve window");
}
