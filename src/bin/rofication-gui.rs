//! rofication-gui: show the queued notifications in rofi.
#![forbid(unsafe_code)]

use rofication::client::Client;
use rofication::gui::Gui;

fn main() {
    // The viewer takes no arguments.
    if let Err(err) = Gui::new(Client::default()).run() {
        rofication::die(err.message());
    }
}
