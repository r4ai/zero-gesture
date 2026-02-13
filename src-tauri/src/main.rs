// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Application entry point — delegates to [`zero_gesture_lib::run`].
fn main() {
    zero_gesture_lib::run()
}
