//! Rust port of Regolith's rofication notification system.
//!
//! Three binaries share this library: `rofication-daemon` (the
//! `org.freedesktop.Notifications` service plus the Unix-socket server),
//! `rofication-gui` (the rofi viewer) and `rofication-status` (the i3xrocks
//! block). The observable behaviour (D-Bus replies, socket bytes, the list
//! document and the saved files) follows the original Python implementation
//! byte for byte, so several modules reproduce Python semantics on purpose:
//! `pystr` (whitespace, `int()`, float `repr`), `json` (the `json` module's
//! parser and its `ensure_ascii` encoder) and `bigint` (unbounded ids).
#![forbid(unsafe_code)]

pub mod bigint;
pub mod client;
pub mod dbus;
pub mod gui;
pub mod json;
pub mod notification;
pub mod pystr;
pub mod queue;
pub mod resource;
pub mod server;
pub mod status;
pub mod strftime;

/// The Unix socket the daemon serves and the clients connect to.
pub const SOCKET_PATH: &str = "/tmp/rofi_notification_daemon";

pub const NAME: &str = "rofication";
pub const URL: &str = "https://github.com/regolith-linux/regolith-rofication";
pub const VERSION: &str = "1.2.2";

/// Write a log line to stdout, ignoring a closed or full stdout: log lines
/// are informational only and must never take a binary down.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, $($arg)*);
        let _ = out.flush();
    }};
}

/// Print an error to stderr and exit with status 1, the status an uncaught
/// exception gives in the original implementation.
pub fn die(message: &str) -> ! {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{message}");
    std::process::exit(1)
}
