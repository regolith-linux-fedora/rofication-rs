//! One notification and its JSON form.

use crate::bigint::BigInt;
use crate::json::{self, Json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Low = 0,
    Normal = 1,
    Critical = 2,
}

impl Urgency {
    /// `Urgency(value)` for an integer value.
    pub fn from_int(value: &BigInt) -> Option<Urgency> {
        match value.to_i64()? {
            0 => Some(Urgency::Low),
            1 => Some(Urgency::Normal),
            2 => Some(Urgency::Critical),
            _ => None,
        }
    }

    /// `Urgency(value)` for a JSON value: the enum looks values up by
    /// equality, so `1.0` and `true` are NORMAL; anything else is invalid.
    pub fn from_json(value: &Json) -> Option<Urgency> {
        match value {
            Json::Int(i) => Urgency::from_int(i),
            Json::Bool(false) => Some(Urgency::Low),
            Json::Bool(true) => Some(Urgency::Normal),
            Json::Float(f) if *f == 0.0 => Some(Urgency::Low),
            Json::Float(f) if *f == 1.0 => Some(Urgency::Normal),
            Json::Float(f) if *f == 2.0 => Some(Urgency::Critical),
            _ => None,
        }
    }
}

/// `tuple(value)`: arrays give their items, strings their characters and
/// objects their keys; scalars are not iterable.
pub fn tuple_of(value: &Json) -> Option<Vec<Json>> {
    match value {
        Json::Array(items) => Some(items.clone()),
        Json::Str(text) => Some(text.chars().map(|c| Json::Str(c.to_string())).collect()),
        Json::Object(map) => Some(map.iter().map(|(k, _)| Json::Str(k.clone())).collect()),
        _ => None,
    }
}

/// A queued notification. Every field but `urgency` and `actions` may be
/// absent (a queue file may omit them); absent fields are not listed.
#[derive(Clone, Debug)]
pub struct Notification {
    pub id: BigInt,
    pub deadline: Option<Json>,
    pub summary: Option<Json>,
    pub body: Option<Json>,
    pub application: Option<Json>,
    pub icon: Option<Json>,
    pub urgency: Urgency,
    pub actions: Vec<Json>,
    pub hints: Option<Json>,
    pub timestamp: Option<Json>,
}

impl Notification {
    /// The sender set the spec's `transient` hint: shown, never saved.
    pub fn transient(&self) -> bool {
        match &self.hints {
            Some(hints @ Json::Object(_)) => hints.get("transient").is_some_and(Json::truthy),
            _ => false,
        }
    }

    pub fn application_is(&self, name: &str) -> bool {
        matches!(&self.application, Some(Json::Str(app)) if app == name)
    }

    pub fn application_str(&self) -> Option<&str> {
        match &self.application {
            Some(Json::Str(app)) => Some(app),
            _ => None,
        }
    }

    /// `'default' in actions` (keys and labels alike).
    pub fn has_default_action(&self) -> bool {
        self.actions
            .iter()
            .any(|a| matches!(a, Json::Str(s) if s == "default"))
    }

    /// The listed form: fields in declaration order, absent ones skipped.
    pub fn dump(&self, out: &mut String) {
        out.push_str("{\"id\": ");
        out.push_str(&self.id.to_string());
        let optional = [
            ("deadline", &self.deadline),
            ("summary", &self.summary),
            ("body", &self.body),
            ("application", &self.application),
            ("icon", &self.icon),
        ];
        for (name, value) in optional {
            if let Some(value) = value {
                field(out, name, value);
            }
        }
        out.push_str(", \"urgency\": ");
        out.push(char::from(b'0' + self.urgency as u8));
        out.push_str(", \"actions\": ");
        Json::Array(self.actions.clone()).dump(out);
        if let Some(hints) = &self.hints {
            field(out, "hints", hints);
        }
        if let Some(timestamp) = &self.timestamp {
            field(out, "timestamp", timestamp);
        }
        out.push('}');
    }
}

fn field(out: &mut String, name: &str, value: &Json) {
    out.push_str(", ");
    json::dump_str(name, out);
    out.push_str(": ");
    value.dump(out);
}

/// Dump a sequence of notifications as the list document.
pub fn dump_list<'a>(items: impl Iterator<Item = &'a Notification>) -> String {
    let mut out = String::from("[");
    for (i, n) in items.enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        n.dump(&mut out);
    }
    out.push(']');
    out
}

/// A notification decoded from a queue file, with its raw id.
pub struct Decoded {
    pub id: Option<Json>,
    pub notification: Notification,
}

/// Decode one queue-file entry. `Err` makes the whole file unusable, as an
/// invalid urgency, a non-iterable `actions` or a non-object entry did.
pub fn decode(entry: &Json) -> Result<Decoded, ()> {
    let Json::Object(map) = entry else {
        return Err(());
    };
    let get = |key: &str| match json::object_get(map, key) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    };
    let urgency = match json::object_get(map, "urgency") {
        None => Urgency::Normal,
        Some(value) => Urgency::from_json(value).ok_or(())?,
    };
    let actions = match json::object_get(map, "actions") {
        None => Vec::new(),
        Some(value) => tuple_of(value).ok_or(())?,
    };
    let timestamp = match json::object_get(map, "timestamp") {
        None => Some(Json::Str(String::new())),
        Some(Json::Null) => None,
        Some(value) => Some(value.clone()),
    };
    Ok(Decoded {
        id: get("id"),
        notification: Notification {
            id: BigInt::zero(),
            deadline: get("deadline"),
            summary: get("summary"),
            body: get("body"),
            application: get("application"),
            icon: get("icon"),
            urgency,
            actions,
            hints: get("hints"),
            timestamp,
        },
    })
}
