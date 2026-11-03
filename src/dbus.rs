//! The `org.freedesktop.Notifications` service.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Signature, Type};

use crate::bigint::BigInt;
use crate::json::{self, Json};
use crate::notification::{Notification, Urgency};
use crate::pystr;
use crate::queue::{Abort, Queue, Signals, CLOSED};
use crate::server::{lock_queue, now};

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";

/// The `a{sv}` hints, decoded the way the original listed them: variants
/// unwrapped, booleans as 0/1, byte arrays and structs as lists, object
/// paths and signatures as strings, dict keys as JSON keys. Entry order is
/// the message order; a repeated key keeps its first position.
pub struct Hints(pub Vec<(String, Json)>);

impl Type for Hints {
    const SIGNATURE: &'static Signature = <HashMap<String, OwnedValue> as Type>::SIGNATURE;
}

impl<'de> Deserialize<'de> for Hints {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match (ValueSeed {
            signature: Self::SIGNATURE,
        })
        .deserialize(deserializer)?
        {
            Json::Object(map) => Ok(Hints(map)),
            _ => Err(de::Error::custom("hints must be a{sv}")),
        }
    }
}

/// A D-Bus value of a known signature, as JSON.
struct ValueSeed<'s> {
    signature: &'s Signature,
}

fn int<E>(value: impl Into<i128>) -> Result<Json, E> {
    let value: i128 = value.into();
    Ok(Json::Int(BigInt::from_digits(
        value < 0,
        value.unsigned_abs().to_string().as_bytes(),
    )))
}

impl<'de> DeserializeSeed<'de> for ValueSeed<'_> {
    type Value = Json;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ValueSeed<'_> {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a D-Bus value of signature {}", self.signature)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Json, E> {
        int(u8::from(v))
    }
    fn visit_i8<E: de::Error>(self, v: i8) -> Result<Json, E> {
        int(v)
    }
    fn visit_u8<E: de::Error>(self, v: u8) -> Result<Json, E> {
        int(v)
    }
    fn visit_i16<E: de::Error>(self, v: i16) -> Result<Json, E> {
        int(v)
    }
    fn visit_u16<E: de::Error>(self, v: u16) -> Result<Json, E> {
        int(v)
    }
    fn visit_i32<E: de::Error>(self, v: i32) -> Result<Json, E> {
        int(v)
    }
    fn visit_u32<E: de::Error>(self, v: u32) -> Result<Json, E> {
        int(v)
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Json, E> {
        int(v)
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Json, E> {
        int(v)
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Json, E> {
        Ok(Json::Float(v))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Json, E> {
        Ok(Json::Str(v.to_string()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Json, E> {
        Ok(Json::Str(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        match self.signature {
            Signature::Variant => {
                let inner: Signature = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::custom("variant without a signature"))?;
                seq.next_element_seed(ValueSeed { signature: &inner })?
                    .ok_or_else(|| de::Error::custom("variant without a value"))
            }
            Signature::Array(child) => {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element_seed(ValueSeed {
                    signature: child.signature(),
                })? {
                    items.push(item);
                }
                Ok(Json::Array(items))
            }
            Signature::Structure(fields) => {
                let mut items = Vec::new();
                for field in fields.iter() {
                    if let Some(item) = seq.next_element_seed(ValueSeed { signature: field })? {
                        items.push(item);
                    }
                }
                Ok(Json::Array(items))
            }
            other => Err(de::Error::custom(format!(
                "unexpected sequence for {other}"
            ))),
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let Signature::Dict { value, .. } = self.signature else {
            return Err(de::Error::custom("unexpected dict"));
        };
        let mut entries: Vec<(Key, Json)> = Vec::new();
        while let Some((key, item)) = map.next_entry_seed(
            KeySeed,
            ValueSeed {
                signature: value.signature(),
            },
        )? {
            match entries.iter_mut().find(|(existing, _)| existing.same(&key)) {
                Some(slot) => slot.1 = item,
                None => entries.push((key, item)),
            }
        }
        Ok(Json::Object(
            entries.into_iter().map(|(k, v)| (k.text(), v)).collect(),
        ))
    }
}

/// A dict key with the equality of the Python value it became.
enum Key {
    Str(String),
    Int(i128),
    Float(f64),
}

impl Key {
    fn same(&self, other: &Key) -> bool {
        match (self, other) {
            (Key::Str(a), Key::Str(b)) => a == b,
            (Key::Int(a), Key::Int(b)) => a == b,
            (Key::Float(a), Key::Float(b)) => a == b,
            _ => false,
        }
    }

    /// The key text `json` writes: strings as-is, integers (booleans
    /// included) in decimal, floats as `repr`.
    fn text(&self) -> String {
        match self {
            Key::Str(s) => s.clone(),
            Key::Int(i) => i.to_string(),
            Key::Float(f) => json::float_text(*f),
        }
    }
}

struct KeySeed;

impl<'de> DeserializeSeed<'de> for KeySeed {
    type Value = Key;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Key, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for KeySeed {
    type Value = Key;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a basic D-Bus value")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Key, E> {
        Ok(Key::Int(i128::from(v)))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Key, E> {
        Ok(Key::Int(i128::from(v)))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Key, E> {
        Ok(Key::Int(i128::from(v)))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Key, E> {
        Ok(Key::Float(v))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Key, E> {
        Ok(Key::Str(v.to_string()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Key, E> {
        Ok(Key::Str(v))
    }
}

/// `Urgency(int(value))` for the `urgency` hint.
fn urgency_hint(value: &Json) -> Option<Urgency> {
    let number = match value {
        Json::Int(i) => i.clone(),
        Json::Float(f) if f.is_finite() => {
            BigInt::from_digits(*f < 0.0, format!("{:.0}", f.abs().trunc()).as_bytes())
        }
        Json::Str(s) => pystr::parse_int(s)?,
        _ => return None,
    };
    Urgency::from_int(&number)
}

/// `datetime.now().timestamp()`: whole seconds plus microseconds.
fn timestamp_now() -> f64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    elapsed.as_secs() as f64 + f64::from(elapsed.subsec_micros()) / 1e6
}

/// Records signals for emission once the queue lock is released.
#[derive(Default)]
struct Recorder(Mutex<Vec<(u32, u32)>>);

impl Signals for Recorder {
    fn closed(&self, id: &BigInt, reason: u32) -> Result<(), Abort> {
        let id = id.to_u32().ok_or(Abort)?;
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((id, reason));
        Ok(())
    }

    fn action_invoked(&self, _id: &BigInt, _key: &str) -> Result<(), Abort> {
        Ok(())
    }
}

pub struct Notifications {
    queue: Arc<Mutex<Queue>>,
}

impl Notifications {
    pub fn new(queue: Arc<Mutex<Queue>>) -> Notifications {
        Notifications { queue }
    }
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    /// The queue keeps every notification until the user dismisses it or
    /// the sender closes it.
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "persistence"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[zbus(out_args("name", "vendor", "version", "spec_version"))]
    fn get_server_information(&self) -> (String, String, String, String) {
        (
            crate::NAME.to_string(),
            crate::URL.to_string(),
            crate::VERSION.to_string(),
            "1.2".to_string(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: Hints,
        expire_timeout: i32,
    ) -> zbus::fdo::Result<u32> {
        let timestamp = timestamp_now();
        let deadline =
            (expire_timeout > 0).then(|| Json::Float(now() + f64::from(expire_timeout) / 1000.0));
        let urgency = match json::object_get(&hints.0, "urgency") {
            None => Urgency::Normal,
            Some(value) => urgency_hint(value).ok_or_else(|| {
                zbus::fdo::Error::InvalidArgs(format!("invalid urgency {}", value.dumps()))
            })?,
        };
        let notification = Notification {
            id: BigInt::from_u64(u64::from(replaces_id)),
            deadline,
            summary: Some(Json::Str(summary)),
            body: Some(Json::Str(body)),
            application: Some(Json::Str(app_name)),
            icon: Some(Json::Str(app_icon)),
            urgency,
            actions: actions.into_iter().map(Json::Str).collect(),
            hints: Some(Json::Object(hints.0)),
            timestamp: Some(Json::Float(timestamp)),
        };
        let id = lock_queue(&self.queue).put(notification);
        // The notification is queued even when its id does not fit the
        // reply's uint32 (a counter restored past 2^32 - 1).
        id.to_u32()
            .ok_or_else(|| zbus::fdo::Error::Failed(format!("OverflowError: id {id}")))
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let recorder = Recorder::default();
        let _ = lock_queue(&self.queue).remove(&BigInt::from_u64(u64::from(id)), CLOSED, &recorder);
        let closed = recorder.0.into_inner().unwrap_or_else(|p| p.into_inner());
        for (id, reason) in closed {
            let _ = Self::notification_closed(&emitter, id, reason).await;
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

/// Signals emitted from the socket server's threads.
pub struct BusSignals {
    connection: zbus::blocking::Connection,
}

impl BusSignals {
    pub fn new(connection: zbus::blocking::Connection) -> BusSignals {
        BusSignals { connection }
    }
}

impl Signals for BusSignals {
    fn closed(&self, id: &BigInt, reason: u32) -> Result<(), Abort> {
        let id = id.to_u32().ok_or(Abort)?;
        let _ = self.connection.emit_signal(
            None::<&str>,
            OBJECT_PATH,
            INTERFACE,
            "NotificationClosed",
            &(id, reason),
        );
        Ok(())
    }

    fn action_invoked(&self, id: &BigInt, key: &str) -> Result<(), Abort> {
        let id = id.to_u32().ok_or(Abort)?;
        let _ = self.connection.emit_signal(
            None::<&str>,
            OBJECT_PATH,
            INTERFACE,
            "ActionInvoked",
            &(id, key),
        );
        Ok(())
    }
}

/// Connect to the session bus, export the object and own the name.
pub fn connect(queue: Arc<Mutex<Queue>>) -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()?
        .serve_at(OBJECT_PATH, Notifications::new(queue))?
        .name(BUS_NAME)?
        .build()
}
