#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|arg| arg == "--sck-audio-helper") {
        if let Err(error) = whisperbar_lib::run_sck_audio_helper() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }

    whisperbar_lib::run();
}
