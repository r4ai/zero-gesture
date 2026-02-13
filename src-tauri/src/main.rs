// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Application entry point — delegates to [`mouse_gesture_lib::run`].
fn main() {
    mouse_gesture_lib::run()
}
