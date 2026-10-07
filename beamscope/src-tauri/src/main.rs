// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Must run first: when the Velopack installer / updater / uninstaller launches the exe with
    // a hook argument, this handles it and exits. In a normal launch it returns immediately.
    velopack::VelopackApp::build().run();
    beamscope_lib::run()
}
