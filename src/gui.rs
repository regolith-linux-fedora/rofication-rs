//! The rofi viewer.

use std::borrow::Cow;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

use crate::bigint::BigInt;
use crate::client::{Client, ClientError};
use crate::json::{self, Json};
use crate::notification::Urgency;
use crate::pystr;
use crate::resource::Resource;
use crate::strftime::Stamp;

/// rofi key bindings and their defaults. Each can be overridden with the
/// resource `rofication.notify.<option>` or the environment variable
/// `rofication_notify_<option>` (`-` replaced by `_`). kb-custom-1..4 make
/// rofi exit with codes 10..13, handled in [`Gui::run`].
const KEYBINDINGS: [(&str, &str); 7] = [
    ("kb-accept-entry", "Control+j,Control+m,KP_Enter"),
    ("kb-remove-char-forward", "Control+d"),
    ("kb-delete-entry", ""),
    ("kb-custom-1", "Delete"),       // dismiss notification
    ("kb-custom-2", "Return"),       // mark notification as seen
    ("kb-custom-3", "Alt+r"),        // refresh
    ("kb-custom-4", "Shift+Delete"), // dismiss all notifications of the application
];

const ROFI_COMMAND_START: [&str; 4] = ["-dmenu", "-p", "Notifications", "-markup"];
const ROFI_COMMAND_END: [&str; 9] = [
    "-markup-rows",
    "-sep",
    "\\0",
    "-format",
    "i",
    "-eh",
    "2",
    "-lines",
    "10",
];

const TIMESTAMP_FORMAT: Resource = Resource::new(
    "i3xrocks_notify_timestamp_format",
    "i3xrocks.notify.timestamp.format",
    "",
);

fn keybinding_value(option: &str, default: &'static str) -> String {
    let resource = Resource {
        default: Cow::Borrowed(default),
        xres_name: Cow::Owned(format!("rofication.notify.{option}")),
        env_name: Some(Cow::Owned(format!(
            "rofication_notify_{}",
            option.replace('-', "_")
        ))),
    };
    pystr::strip(&resource.fetch()).to_string()
}

/// `GLib.markup_escape_text`: the five XML specials, and the C0/C1 control
/// characters other than tab, newline, carriage return and NEL as `&#x..;`.
/// The text is a C string there: it ends at the first NUL.
pub fn markup_escape(text: &str) -> String {
    let text = text.split('\0').next().unwrap_or_default();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&#39;"),
            '"' => out.push_str("&quot;"),
            '\u{1}'..='\u{8}'
            | '\u{b}'..='\u{c}'
            | '\u{e}'..='\u{1f}'
            | '\u{7f}'..='\u{84}'
            | '\u{86}'..='\u{9f}' => out.push_str(&format!("&#x{:x};", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

/// Remove `<...>` tags (`<[^>]*?>`): a `<` up to the next `>`; a `<` with
/// no `>` after it stays.
pub fn remove_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        match rest[open..].find('>') {
            Some(close) => {
                out.push_str(&rest[..open]);
                rest = &rest[open + close + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

pub fn strip_tags(text: &str) -> String {
    markup_escape(&remove_tags(text))
}

/// One listed notification, as the viewer uses it.
struct Row {
    id: String,
    summary: String,
    body: String,
    application: String,
    urgency: Urgency,
    timestamp: Option<Json>,
}

fn fatal(message: impl Into<String>) -> ClientError {
    ClientError::Fatal(message.into())
}

/// `str(value)` for an id sent back in a command.
fn py_str(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => "None".to_string(),
        Some(Json::Bool(true)) => "True".to_string(),
        Some(Json::Bool(false)) => "False".to_string(),
        Some(Json::Int(i)) => i.to_string(),
        Some(Json::Float(f)) => pystr::float_repr(*f),
        Some(Json::Str(s)) => s.clone(),
        Some(other) => other.dumps(),
    }
}

/// The list is decoded with `Notification.make` as the object hook of
/// every JSON object, nested hint dicts included: each one needs a valid
/// `urgency` (when present) and an iterable `actions` (when present).
fn check_object_hook(value: &Json) -> Result<(), ClientError> {
    match value {
        Json::Array(items) => items.iter().try_for_each(check_object_hook),
        Json::Object(map) => {
            for (_, inner) in map {
                check_object_hook(inner)?;
            }
            if let Some(urgency) = json::object_get(map, "urgency") {
                if Urgency::from_json(urgency).is_none() {
                    return Err(fatal("ValueError: invalid urgency in list reply"));
                }
            }
            if let Some(actions) = json::object_get(map, "actions") {
                // Nested objects are already notifications: not iterable.
                if !matches!(actions, Json::Array(_) | Json::Str(_)) {
                    return Err(fatal("TypeError: actions is not iterable"));
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn rows(document: &Json) -> Result<Vec<Row>, ClientError> {
    check_object_hook(document)?;
    let Json::Array(items) = document else {
        return Err(fatal("TypeError: list reply is not an array"));
    };
    let text = |map: &[(String, Json)], key: &str| match json::object_get(map, key) {
        Some(Json::Str(s)) => Ok(s.clone()),
        _ => Err(fatal(format!("TypeError: {key} is not a string"))),
    };
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(map) = item else {
            return Err(fatal("AttributeError: list entry is not an object"));
        };
        let urgency = match json::object_get(map, "urgency") {
            Some(u) => Urgency::from_json(u).unwrap_or(Urgency::Normal),
            None => Urgency::Normal,
        };
        let timestamp = match json::object_get(map, "timestamp") {
            None => Some(Json::Str(String::new())),
            Some(Json::Null) => None,
            Some(t) => Some(t.clone()),
        };
        rows.push(Row {
            id: py_str(json::object_get(map, "id")),
            summary: text(map, "summary")?,
            application: text(map, "application")?,
            body: text(map, "body")?,
            urgency,
            timestamp,
        });
    }
    Ok(rows)
}

fn format_timestamp(timestamp: &Option<Json>, format: &str) -> Result<String, ClientError> {
    let stamp = match timestamp {
        Some(Json::Float(f)) => Stamp::from_float(*f),
        Some(Json::Int(i)) => i.to_i64().and_then(|s| Stamp::from_parts(s, 0)),
        Some(Json::Bool(b)) => Stamp::from_parts(i64::from(*b), 0),
        _ => None,
    };
    stamp
        .map(|s| s.format(format))
        .ok_or_else(|| fatal("invalid notification timestamp"))
}

fn rofi_entry(row: &Row, ts_format: &str) -> Result<String, ClientError> {
    let summary = strip_tags(&row.summary);
    let application = strip_tags(&row.application);
    let body = strip_tags(
        &pystr::split_whitespace(&row.body)
            .collect::<Vec<_>>()
            .join(" "),
    );
    let prefix = if ts_format.is_empty() {
        String::new()
    } else {
        format!("{} ", format_timestamp(&row.timestamp, ts_format)?)
    };
    Ok(format!(
        "<b>{prefix}{summary}</b> <small>({application})</small>\n<small>{body}</small>"
    ))
}

/// Run rofi with `entries` on stdin (NUL-separated); returns the selected
/// row (-1 when rofi printed nothing) and rofi's exit status.
fn call_rofi(entries: &[String], argv: &[String]) -> Result<(BigInt, i32), ClientError> {
    let mut child = Command::new("rofi")
        .args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| ClientError::from_io("rofi", e))?;
    {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let mut payload = Vec::new();
        for entry in entries {
            payload.extend_from_slice(entry.as_bytes());
            payload.push(0);
        }
        stdin
            .write_all(&payload)
            .and_then(|()| stdin.flush())
            .map_err(|e| fatal(format!("BrokenPipeError: {e}")))?;
    }
    let mut selected = Vec::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_end(&mut selected)
        .map_err(|e| fatal(format!("rofi output: {e}")))?;
    let status = child.wait().map_err(|e| fatal(format!("rofi: {e}")))?;
    let code = match status.code() {
        Some(code) => code,
        None => {
            use std::os::unix::process::ExitStatusExt as _;
            -status.signal().unwrap_or(0)
        }
    };
    let selected =
        String::from_utf8(selected).map_err(|_| fatal("rofi printed text that is not UTF-8"))?;
    if selected.is_empty() {
        return Ok((BigInt::from_i64(-1), code));
    }
    let row = pystr::parse_int(&selected)
        .ok_or_else(|| fatal(format!("ValueError: invalid row {selected:?}")))?;
    Ok((row, code))
}

pub struct Gui {
    client: Client,
    ts_format: String,
    keybindings: Vec<(&'static str, String)>,
}

impl Gui {
    /// Resolve the timestamp format, then the seven key bindings.
    pub fn new(client: Client) -> Gui {
        let ts_format = TIMESTAMP_FORMAT.fetch();
        let keybindings = KEYBINDINGS
            .iter()
            .map(|(option, default)| (*option, keybinding_value(option, default)))
            .collect();
        Gui {
            client,
            ts_format,
            keybindings,
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    fn rofi_argv(&self, extra: &[String]) -> Vec<String> {
        let mut argv: Vec<String> = ROFI_COMMAND_START.iter().map(|s| s.to_string()).collect();
        for (option, binding) in &self.keybindings {
            argv.push(format!("-{option}"));
            argv.push(binding.clone());
        }
        argv.extend(ROFI_COMMAND_END.iter().map(|s| s.to_string()));
        argv.extend_from_slice(extra);
        argv
    }

    pub fn run(&self) -> Result<(), ClientError> {
        let zero = BigInt::zero();
        let mut selected = zero.clone();
        while selected >= zero {
            let rows = rows(&self.client.list()?)?;
            let mut entries = Vec::with_capacity(rows.len());
            let mut urgent = Vec::new();
            let mut low = Vec::new();
            for (index, row) in rows.iter().enumerate() {
                entries.push(rofi_entry(row, &self.ts_format)?);
                match row.urgency {
                    Urgency::Critical => urgent.push(index.to_string()),
                    Urgency::Low => low.push(index.to_string()),
                    Urgency::Normal => {}
                }
            }
            let mut args = Vec::new();
            if !urgent.is_empty() {
                args.push("-u".to_string());
                args.push(urgent.join(","));
            }
            if !low.is_empty() {
                args.push("-a".to_string());
                args.push(low.join(","));
            }
            args.push("-selected-row".to_string());
            args.push(selected.to_string());

            let (row, exit_code) = call_rofi(&entries, &self.rofi_argv(&args))?;
            selected = row;
            if selected < zero {
                break;
            }
            let pick = || {
                selected
                    .to_i64()
                    .and_then(|i| usize::try_from(i).ok())
                    .and_then(|i| rows.get(i))
                    .ok_or_else(|| fatal("IndexError: list index out of range"))
            };
            match exit_code {
                // Dismiss the notification.
                10 => {
                    self.client.delete(&pick()?.id)?;
                    if rows.len() == 1 {
                        break;
                    }
                }
                // Mark it seen.
                11 => self.client.see(&pick()?.id)?,
                // Dismiss all notifications of its application.
                13 => {
                    self.client.delete_all(&pick()?.application)?;
                    if rows.len() == 1 {
                        break;
                    }
                }
                // Refresh.
                12 => {}
                _ => break,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_and_escaping() {
        assert_eq!(
            strip_tags("Hello <b>World</b>  spaced"),
            "Hello World  spaced"
        );
        assert_eq!(strip_tags("A<B>C&D\"q\""), "AC&amp;D&quot;q&quot;");
        assert_eq!(strip_tags("broken <tag and > literal"), "broken  literal");
        assert_eq!(strip_tags("a < b"), "a &lt; b");
        assert_eq!(strip_tags("x<y<z>w"), "xw");
        assert_eq!(
            strip_tags("\u{7}\u{9}\u{85}\u{86}'"),
            "&#x7;\t\u{85}&#x86;&#39;"
        );
        // Tags go first; escaping then stops at the first NUL.
        assert_eq!(strip_tags("a<\0>b\0c"), "ab");
    }
}
