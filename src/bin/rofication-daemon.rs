//! rofication-daemon: the notification daemon.
//!
//! Owns `org.freedesktop.Notifications` on the session bus, serves the
//! queue on `/tmp/rofi_notification_daemon`, and saves it to
//! `~/.cache/rofication/` when SIGTERM, SIGINT or SIGHUP stops it.
#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use rofication::queue::{parse_app_list, Queue, SINGLE_NOTIFICATION_APPS};
use rofication::resource::Resource;
use rofication::server::{lock_queue, Server};
use rofication::{dbus, die, log, SOCKET_PATH};

const SINGLE_APPS: Resource = Resource::new(
    "rofication_notify_single_notification_apps",
    "rofication.notify.single_notification_apps",
    SINGLE_NOTIFICATION_APPS,
);

/// `os.path.expanduser("~/.cache/rofication")`.
fn queue_dir() -> PathBuf {
    let home = match env::var_os("HOME") {
        Some(home) => home.to_string_lossy().into_owned(),
        #[allow(deprecated)]
        None => env::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
    };
    PathBuf::from(format!("{}/.cache/rofication", home.trim_end_matches('/')))
}

fn main() {
    let dir = queue_dir();
    if let Err(err) = fs::create_dir_all(&dir) {
        die(&format!("cannot create {}: {err}", dir.display()));
    }
    let queue_file = dir.join("notifications.json");
    // Resource lookups run here, on the main thread, before any thread
    // exists.
    let queue = Queue::load(&queue_file, || parse_app_list(&SINGLE_APPS.fetch()));
    let queue = Arc::new(Mutex::new(queue));

    // Stop on the signals a session or the user sends to end the daemon,
    // so the queue gets saved.
    let mut signals = Signals::new([SIGTERM, SIGINT, SIGHUP])
        .unwrap_or_else(|err| die(&format!("cannot install signal handlers: {err}")));

    let connection = dbus::connect(Arc::clone(&queue))
        .unwrap_or_else(|err| die(&format!("cannot serve {}: {err}", dbus::BUS_NAME)));

    let mut server = Server::bind(
        Path::new(SOCKET_PATH),
        Arc::clone(&queue),
        Box::new(dbus::BusSignals::new(connection.clone())),
    )
    .unwrap_or_else(|err| die(&format!("cannot listen on {SOCKET_PATH}: {err}")));
    server.start();

    let signum = signals.forever().next().unwrap_or(SIGTERM);
    log!("Received signal {signum}, stopping");
    server.stop();
    lock_queue(&queue).save(&queue_file);
    std::process::exit(0);
}
