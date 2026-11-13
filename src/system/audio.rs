use std::path::Path;

use gtk4::gio;
use tracing::debug;

const DEFAULT_START_SOUND: &str = "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga";
const DEFAULT_END_SOUND: &str = "/usr/share/sounds/freedesktop/stereo/complete.oga";
const FLATPAK_START_SOUND: &str = "/app/share/safeeyes-rs/sounds/break-start.wav";
const FLATPAK_END_SOUND: &str = "/app/share/safeeyes-rs/sounds/break-end.wav";

/// Plays a subtle chime asynchronously using `gio::Subprocess` (zero idle RAM/CPU overhead).
pub fn play_break_start(enabled: bool, custom_path: Option<&str>) {
    if !enabled {
        return;
    }
    play_sound(
        custom_path,
        &[DEFAULT_START_SOUND, FLATPAK_START_SOUND],
        "message-new-instant",
    );
}

pub fn play_break_end(enabled: bool, custom_path: Option<&str>) {
    if !enabled {
        return;
    }
    play_sound(
        custom_path,
        &[DEFAULT_END_SOUND, FLATPAK_END_SOUND],
        "complete",
    );
}

fn play_sound(custom_path: Option<&str>, fallback_files: &[&str], canberra_id: &str) {
    if let Some(path) = custom_path
        && !path.is_empty()
        && Path::new(path).exists()
        && spawn_audio_file(path)
    {
        return;
    }

    for file in fallback_files {
        if Path::new(file).exists() && spawn_audio_file(file) {
            return;
        }
    }

    // Fallback to freedesktop sound theme event ID via canberra-gtk-play.
    if spawn_process(&["canberra-gtk-play", "-i", canberra_id]) {
        return;
    }

    debug!("no audio player available to play chime");
}

fn spawn_audio_file(path: &str) -> bool {
    spawn_process(&["pw-play", path]) || spawn_process(&["paplay", path])
}

fn spawn_process(argv: &[&str]) -> bool {
    let os_args: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    match gio::Subprocess::newv(
        &os_args,
        gio::SubprocessFlags::STDOUT_SILENCE | gio::SubprocessFlags::STDERR_SILENCE,
    ) {
        Ok(proc) => {
            // Reap asynchronously on GLib main context without blocking.
            proc.wait_async(gio::Cancellable::NONE, |_| {});
            true
        }
        Err(_) => false,
    }
}
