//! rofication-status: the i3xrocks notification block.
#![forbid(unsafe_code)]

use std::io::Write as _;

fn main() {
    match rofication::status::render() {
        Ok(line) => {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
        Err(message) => rofication::die(&message),
    }
}
