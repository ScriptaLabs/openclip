// A release build is a windowed app. Without this Windows opens a console beside it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    openclip_app_lib::run();
}
