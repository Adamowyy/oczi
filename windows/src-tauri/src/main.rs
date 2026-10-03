// Oczi runs without a console window: Iskra is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    oczi_lib::run()
}
