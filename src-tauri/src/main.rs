// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Application entry point — selects the Engine or Settings process mode.
fn main() {
    if let Err(error) = zero_gesture_lib::run_from_args(std::env::args_os()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
