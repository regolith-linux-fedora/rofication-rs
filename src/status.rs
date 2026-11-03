//! The i3xrocks status block.

use std::env;

use crate::bigint::BigInt;
use crate::client::{Client, ClientError};
use crate::gui::Gui;
use crate::resource::Resource;

static VALUE_FONT: Resource =
    Resource::new("font", "i3xrocks.value.font", "Source Code Pro Medium 13");
static NOTIFY_NONE: Resource = Resource::new(
    "i3xrocks_label_notify_none",
    "i3xrocks.label.notify.none",
    "N",
);
static NOTIFY_SOME: Resource = Resource::new(
    "i3xrocks_label_notify_some",
    "i3xrocks.label.notify.some",
    "N",
);
static NOTIFY_ERROR: Resource = Resource::new(
    "i3xrocks_label_notify_error",
    "i3xrocks.label.notify.error",
    "N",
);
static VALUE_COLOR: Resource = Resource::new("color", "i3xrocks.value.color", "#E6E1CF");
static LABEL_COLOR: Resource = Resource::new("label_color", "i3xrocks.label.color", "#E6E1CF");
static WARNING_COLOR: Resource = Resource::new("warn_color", "i3xrocks.warning", "#FFD580");
static CRITICAL_COLOR: Resource = Resource::xres_only("i3xrocks.critical.color", "#BF616A");

fn count(client: Client) -> Result<(BigInt, BigInt), ClientError> {
    // A set `button` (the block was clicked) opens the viewer first.
    if env::var_os("button").is_some_and(|b| !b.is_empty()) {
        let gui = Gui::new(client);
        gui.run()?;
        gui.client().count()
    } else {
        client.count()
    }
}

/// The block's single output line (without the newline), or the message
/// of a failure that takes the block down.
pub fn render() -> Result<String, String> {
    let mut label_icon = &NOTIFY_NONE;
    let mut label_color = &LABEL_COLOR;
    let mut value_color = &VALUE_COLOR;
    let num = match count(Client::default()) {
        Ok((num, crit)) => {
            let zero = BigInt::zero();
            if num > zero {
                label_icon = &NOTIFY_SOME;
                value_color = &WARNING_COLOR;
            }
            if crit > zero {
                value_color = &CRITICAL_COLOR;
            }
            num.to_string()
        }
        Err(ClientError::Down(_)) => {
            label_icon = &NOTIFY_ERROR;
            label_color = &CRITICAL_COLOR;
            "?".to_string()
        }
        Err(ClientError::Fatal(message)) => return Err(message),
    };
    // Resources are fetched only now, in this order.
    let label_color = label_color.fetch();
    let label_icon = label_icon.fetch();
    let value_font = VALUE_FONT.fetch();
    let value_color = value_color.fetch();
    Ok(format!(
        "<span foreground=\"{label_color}\">{label_icon}</span>\
         <span font_desc=\"{value_font}\" foreground=\"{value_color}\"> {num}</span>"
    ))
}
