//! The socket server and client together, over a real Unix socket.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rofication::bigint::BigInt;
use rofication::client::Client;
use rofication::json::{self, Json};
use rofication::notification::{Notification, Urgency};
use rofication::queue::{Abort, Queue, Signals};
use rofication::server::{lock_queue, Server};

struct NoSignals;

impl Signals for NoSignals {
    fn closed(&self, _id: &BigInt, _reason: u32) -> Result<(), Abort> {
        Ok(())
    }
    fn action_invoked(&self, _id: &BigInt, _key: &str) -> Result<(), Abort> {
        Ok(())
    }
}

fn note(application: &str, hints: &str) -> Notification {
    Notification {
        id: BigInt::zero(),
        deadline: None,
        summary: Some(Json::Str("summary".into())),
        body: Some(Json::Str("body".into())),
        application: Some(Json::Str(application.into())),
        icon: Some(Json::Str(String::new())),
        urgency: Urgency::Normal,
        actions: Vec::new(),
        hints: Some(json::parse(hints).unwrap()),
        timestamp: Some(Json::Float(1_700_000_000.5)),
    }
}

struct Daemon {
    dir: PathBuf,
    server: Server,
    queue: Arc<Mutex<Queue>>,
}

impl Daemon {
    fn start(name: &str, notes: Vec<Notification>) -> Daemon {
        let dir =
            std::env::temp_dir().join(format!("rofication-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut queue = Queue::new(Vec::new(), Vec::new(), BigInt::from_u64(1));
        for n in notes {
            queue.put(n);
        }
        let queue = Arc::new(Mutex::new(queue));
        let mut server =
            Server::bind(&dir.join("socket"), Arc::clone(&queue), Box::new(NoSignals)).unwrap();
        server.start();
        Daemon { dir, server, queue }
    }

    fn client(&self) -> Client {
        Client::new(self.dir.join("socket").to_str().unwrap())
    }

    fn applications(&self) -> Vec<String> {
        lock_queue(&self.queue)
            .iter()
            .map(|n| n.application_str().unwrap().to_string())
            .collect()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.server.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn dismiss_by_application_name_with_a_colon() {
    let daemon = Daemon::start(
        "dela",
        vec![
            note("Chat", "{}"),
            note("Chat: Work", "{}"),
            note("Chat: Work: Urgent", "{}"),
        ],
    );
    daemon.client().delete_all("Chat: Work").unwrap();
    // The request is applied before the connection closes; a following
    // request sees it.
    daemon.client().count().unwrap();
    assert_eq!(daemon.applications(), ["Chat", "Chat: Work: Urgent"]);

    daemon.client().delete_all("Chat").unwrap();
    daemon.client().count().unwrap();
    assert_eq!(daemon.applications(), ["Chat: Work: Urgent"]);
}
