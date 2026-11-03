//! The notification queue: ids, replacement rules, expiry and persistence.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::bigint::BigInt;
use crate::json::{self, Json};
use crate::notification::{self, Notification, Urgency};
use crate::pystr;

/// `NotificationClosed` reasons.
pub const EXPIRED: u32 = 1;
pub const DISMISSED: u32 = 2;
pub const CLOSED: u32 = 3;

/// Applications whose notifications expire at their deadline (transient
/// notifications of any application expire too).
const ALLOWED_TO_EXPIRE: &[&str] = &["notify-send"];

/// Default single-notification applications (`, `-joined as the default of
/// the `single_notification_apps` resource).
pub const SINGLE_NOTIFICATION_APPS: &str = "VLC media player";

/// The current operation stops here: a signal could not be emitted (an id a
/// D-Bus `uint32` cannot carry, or a deadline that is not a number).
#[derive(Debug)]
pub struct Abort;

/// Receiver of the queue's D-Bus signals, emitted in queue order.
pub trait Signals {
    fn closed(&self, id: &BigInt, reason: u32) -> Result<(), Abort>;
    fn action_invoked(&self, id: &BigInt, key: &str) -> Result<(), Abort>;
}

/// Split a comma-separated list of application names.
pub fn parse_app_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(pystr::strip)
        .filter(|app| !app.is_empty())
        .map(str::to_string)
        .collect()
}

/// The file next to the queue file that keeps the next notification id.
pub fn next_id_path(queue_file: &Path) -> PathBuf {
    queue_file.with_extension("next-id")
}

pub struct Queue {
    single_apps: Vec<String>,
    /// The next id to hand out; never decreases, also across restarts.
    last_id: BigInt,
    /// Insertion order is list order; ids are unique.
    entries: Vec<Notification>,
}

impl Queue {
    pub fn new(entries: Vec<Notification>, single_apps: Vec<String>, next_id: BigInt) -> Queue {
        let mut last_id = next_id;
        for n in &entries {
            let candidate = n.id.succ();
            if candidate > last_id {
                last_id = candidate;
            }
        }
        Queue {
            single_apps,
            last_id,
            entries,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn critical_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|n| n.urgency == Urgency::Critical)
            .count()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Notification> {
        self.entries.iter()
    }

    fn position(&self, id: &BigInt) -> Option<usize> {
        self.entries.iter().position(|n| n.id == *id)
    }

    /// Queue a notification; `notification.id` holds the requested
    /// `replaces_id`. Returns the id it was stored under.
    pub fn put(&mut self, mut notification: Notification) -> BigInt {
        let single = notification
            .application_str()
            .is_some_and(|app| self.single_apps.iter().any(|s| s == app));
        let to_replace = if single {
            // Applications that keep one notification replace their first.
            self.entries
                .iter()
                .find(|n| n.application == notification.application)
                .map(|n| n.id.clone())
        } else {
            self.position(&notification.id)
                .map(|_| notification.id.clone())
        };
        match to_replace {
            // Id 0 is falsy: it never replaces.
            Some(id) if !id.is_zero() => {
                notification.id = id.clone();
                crate::log!("Replacing: {id}");
                let at = self.position(&id).expect("replaced id is queued");
                self.entries[at] = notification;
                id
            }
            _ => {
                let id = self.last_id.clone();
                self.last_id = id.succ();
                notification.id = id.clone();
                crate::log!("Adding: {id}");
                self.entries.push(notification);
                id
            }
        }
    }

    pub fn see(&mut self, id: &BigInt, signals: &dyn Signals) -> Result<(), Abort> {
        let Some(at) = self.position(id) else {
            crate::log!("Unable to find notification {id}");
            return Ok(());
        };
        crate::log!("Seeing: {id}");
        let notification = &mut self.entries[at];
        notification.urgency = Urgency::Normal;
        if notification.has_default_action() {
            signals.action_invoked(id, "default")?;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &BigInt, reason: u32, signals: &dyn Signals) -> Result<(), Abort> {
        let Some(at) = self.position(id) else {
            crate::log!("Unable to find notification {id}");
            return Ok(());
        };
        crate::log!("Removing: {id}");
        let notification = self.entries.remove(at);
        signals.closed(&notification.id, reason)
    }

    pub fn remove_all(
        &mut self,
        ids: &[BigInt],
        reason: u32,
        signals: &dyn Signals,
    ) -> Result<(), Abort> {
        for id in ids {
            self.remove(id, reason, signals)?;
        }
        Ok(())
    }

    pub fn ids_of_application(&self, application: &str) -> Vec<BigInt> {
        self.entries
            .iter()
            .filter(|n| n.application_is(application))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Drop expired notifications, emitting `NotificationClosed(id, 1)`
    /// for each before it goes.
    pub fn cleanup(&mut self, now: f64, signals: &dyn Signals) -> Result<(), Abort> {
        let mut expired = Vec::new();
        for n in &self.entries {
            let Some(deadline) = &n.deadline else {
                continue;
            };
            if !deadline.truthy() {
                continue;
            }
            let past = match deadline {
                Json::Int(i) => i.to_f64() < now,
                Json::Float(f) => *f < now,
                Json::Bool(_) => 1.0 < now,
                // Comparing a string, list or dict with a float raises.
                _ => return Err(Abort),
            };
            if past && (ALLOWED_TO_EXPIRE.iter().any(|app| n.application_is(app)) || n.transient())
            {
                expired.push(n.id.clone());
            }
        }
        if !expired.is_empty() {
            let shown: Vec<String> = expired.iter().map(BigInt::to_string).collect();
            crate::log!("Expired: [{}]", shown.join(", "));
        }
        for id in expired {
            signals.closed(&id, EXPIRED)?;
            if let Some(at) = self.position(&id) {
                self.entries.remove(at);
            }
        }
        Ok(())
    }

    pub fn list_json(&self) -> String {
        notification::dump_list(self.entries.iter())
    }

    /// Save the queue (transient notifications excluded) and the next id.
    pub fn save(&self, queue_file: &Path) {
        crate::log!("Saving notification queue to file");
        let document = notification::dump_list(self.entries.iter().filter(|n| !n.transient()));
        let written =
            fs::File::create(queue_file).and_then(|mut f| f.write_all(document.as_bytes()));
        if written.is_err() {
            eprintln_ignore("Failed to save notification queue");
            if queue_file.exists() {
                let _ = fs::remove_file(queue_file);
            }
        }
        let next = format!("{}\n", self.last_id);
        let written = fs::File::create(next_id_path(queue_file))
            .and_then(|mut f| f.write_all(next.as_bytes()));
        if written.is_err() {
            eprintln_ignore("Failed to save the next notification ID");
        }
    }

    /// Load the saved queue. `single_apps` is called once the files have
    /// been read, as the queue is constructed. A queue file that cannot be
    /// read or decoded gives an empty queue; the saved next id still counts.
    pub fn load(queue_file: &Path, single_apps: impl FnOnce() -> Vec<String>) -> Queue {
        let next_id = fs::read(next_id_path(queue_file))
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| pystr::parse_int(&pystr::translate_newlines(&text)))
            .unwrap_or_else(|| BigInt::from_u64(1));

        if !queue_file.exists() {
            crate::log!("Creating empty notification queue");
            return Queue::new(Vec::new(), single_apps(), next_id);
        }

        crate::log!("Loading notification queue from file");
        let entries = match decode_queue_file(queue_file) {
            Ok(entries) => entries,
            Err(()) => {
                eprintln_ignore("Failed to load notification queue");
                Vec::new()
            }
        };
        Queue::new(entries, single_apps(), next_id)
    }
}

fn decode_queue_file(queue_file: &Path) -> Result<Vec<Notification>, ()> {
    let bytes = fs::read(queue_file).map_err(|_| ())?;
    let text = String::from_utf8(bytes).map_err(|_| ())?;
    let document = json::parse(&text).map_err(|_| ())?;
    // Iterating the root: an array yields its entries; an empty string or
    // object yields nothing; anything else fails.
    let items: &[Json] = match &document {
        Json::Array(items) => items,
        Json::Str(s) if s.is_empty() => &[],
        Json::Object(m) if m.is_empty() => &[],
        _ => return Err(()),
    };
    let decoded = items
        .iter()
        .map(notification::decode)
        .collect::<Result<Vec<_>, ()>>()?;
    // {n.id: n for n in notifications if not n.transient}
    let mut entries: Vec<Notification> = Vec::new();
    for d in decoded {
        if d.notification.transient() {
            continue;
        }
        let id = match d.id {
            Some(Json::Int(id)) => id,
            // Unhashable ids make the dict comprehension fail.
            Some(Json::Array(_)) | Some(Json::Object(_)) => return Err(()),
            // `None + 1` / `str + 1` abort the daemon when the queue is built.
            _ => crate::die("TypeError: notification ids must be integers"),
        };
        let mut n = d.notification;
        n.id = id;
        match entries.iter_mut().find(|e| e.id == n.id) {
            Some(slot) => *slot = n,
            None => entries.push(n),
        }
    }
    Ok(entries)
}

fn eprintln_ignore(message: &str) {
    let _ = writeln!(std::io::stderr(), "{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder(std::cell::RefCell<Vec<(String, u32)>>);

    impl Signals for Recorder {
        fn closed(&self, id: &BigInt, reason: u32) -> Result<(), Abort> {
            self.0.borrow_mut().push((id.to_string(), reason));
            Ok(())
        }
        fn action_invoked(&self, id: &BigInt, _key: &str) -> Result<(), Abort> {
            self.0.borrow_mut().push((id.to_string(), 0));
            Ok(())
        }
    }

    fn note(app: &str, replaces: u64) -> Notification {
        Notification {
            id: BigInt::from_u64(replaces),
            deadline: None,
            summary: Some(Json::Str("s".into())),
            body: Some(Json::Str("b".into())),
            application: Some(Json::Str(app.into())),
            icon: Some(Json::Str(String::new())),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            hints: Some(Json::Object(Vec::new())),
            timestamp: Some(Json::Float(1.5)),
        }
    }

    #[test]
    fn ids_and_replacement() {
        let mut q = Queue::new(
            Vec::new(),
            parse_app_list("VLC media player"),
            BigInt::from_u64(1),
        );
        assert_eq!(q.put(note("a", 0)).to_string(), "1");
        assert_eq!(q.put(note("b", 0)).to_string(), "2");
        assert_eq!(q.put(note("c", 1)).to_string(), "1");
        assert_eq!(q.put(note("c", 99)).to_string(), "3");
        assert_eq!(q.put(note("VLC media player", 0)).to_string(), "4");
        assert_eq!(q.put(note("VLC media player", 2)).to_string(), "4");
        let apps: Vec<_> = q
            .iter()
            .map(|n| n.application_str().unwrap().to_string())
            .collect();
        assert_eq!(apps, ["c", "b", "c", "VLC media player"]);
    }

    #[test]
    fn app_list() {
        assert_eq!(parse_app_list("  A , B "), ["A", "B"]);
        assert!(parse_app_list(" , ").is_empty());
    }

    #[test]
    fn removal_signals() {
        let mut q = Queue::new(Vec::new(), Vec::new(), BigInt::from_u64(1));
        q.put(note("a", 0));
        q.put(note("a", 0));
        let rec = Recorder(Default::default());
        q.remove_all(&q.ids_of_application("a"), DISMISSED, &rec)
            .unwrap();
        assert_eq!(
            *rec.0.borrow(),
            [("1".to_string(), 2), ("2".to_string(), 2)]
        );
        assert!(q.is_empty());
    }
}
