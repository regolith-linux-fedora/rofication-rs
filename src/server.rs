//! The Unix-socket server: one request line per connection, one thread per
//! connection, expiry cleanup before every request.

use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::bigint::BigInt;
use crate::pystr;
use crate::queue::{Queue, Signals, DISMISSED};

/// How long the accept loop waits for a request to finish before it serves
/// the next connection concurrently.
const IN_ORDER_WAIT: Duration = Duration::from_millis(200);

/// `time.time()`.
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn lock_queue(queue: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// (st_dev, st_ino, st_ctime_ns): the socket file is ours only while all
/// three still match.
type FileIdentity = (u64, u64, i128);

fn file_identity(path: &Path) -> std::io::Result<FileIdentity> {
    let meta = fs::metadata(path)?;
    Ok((
        meta.dev(),
        meta.ino(),
        i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
    ))
}

struct Shared {
    queue: Arc<Mutex<Queue>>,
    signals: Box<dyn Signals + Send + Sync>,
    stopping: AtomicBool,
    active: Mutex<usize>,
    idle: Condvar,
}

/// Decrements the in-flight request count when a handler ends.
struct ActiveGuard<'a>(&'a Shared);

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        let mut active = self.0.active.lock().unwrap_or_else(|p| p.into_inner());
        *active -= 1;
        self.0.idle.notify_all();
    }
}

pub struct Server {
    path: PathBuf,
    identity: FileIdentity,
    shared: Arc<Shared>,
    listener: Option<UnixListener>,
}

impl Server {
    /// Replace any file at `path` with a fresh listening socket.
    pub fn bind(
        path: &Path,
        queue: Arc<Mutex<Queue>>,
        signals: Box<dyn Signals + Send + Sync>,
    ) -> std::io::Result<Server> {
        if path.exists() {
            fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        let identity = file_identity(path)?;
        Ok(Server {
            path: path.to_path_buf(),
            identity,
            shared: Arc::new(Shared {
                queue,
                signals,
                stopping: AtomicBool::new(false),
                active: Mutex::new(0),
                idle: Condvar::new(),
            }),
            listener: Some(listener),
        })
    }

    /// Serve connections on a background thread.
    pub fn start(&mut self) {
        let Some(listener) = self.listener.take() else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        let _ = thread::Builder::new()
            .name("rofication-socket".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else {
                        continue;
                    };
                    if shared.stopping.load(Ordering::SeqCst) {
                        continue;
                    }
                    *shared.active.lock().unwrap_or_else(|p| p.into_inner()) += 1;
                    let worker = Arc::clone(&shared);
                    let (done_tx, done_rx) = mpsc::channel::<()>();
                    let spawned = thread::Builder::new()
                        .name("rofication-request".into())
                        .spawn(move || {
                            let _guard = ActiveGuard(&worker);
                            handle(&worker, stream);
                            let _ = done_tx.send(());
                        });
                    match spawned {
                        // Requests are served in arrival order: a burst of
                        // commands (lago clear, then a list) is applied in
                        // the order the clients connected. Only a client
                        // that stalls lets the next connection overtake it.
                        Ok(_) => {
                            let _ = done_rx.recv_timeout(IN_ORDER_WAIT);
                        }
                        Err(_) => {
                            let mut active =
                                shared.active.lock().unwrap_or_else(|p| p.into_inner());
                            *active -= 1;
                        }
                    }
                }
            });
    }

    /// Stop serving: finish the requests in flight (bounded), then remove
    /// the socket file unless it is gone or no longer ours (a daemon started
    /// since then owns the path).
    pub fn stop(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut active = self.shared.active.lock().unwrap_or_else(|p| p.into_inner());
        while *active > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            active = self
                .shared
                .idle
                .wait_timeout(active, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        drop(active);
        if file_identity(&self.path).is_ok_and(|id| id == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Read one line as a universal-newlines UTF-8 text reader does: it ends
/// at `\n`, `\r` or EOF; a `\r` at the end of the data read so far waits
/// for the next byte (it may start `\r\n`). `None`: invalid UTF-8 or a
/// read error.
fn read_line(stream: &mut UnixStream) -> Option<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut scanned = 0;
    loop {
        let mut end = None;
        while scanned < buf.len() {
            match buf[scanned] {
                b'\n' => {
                    end = Some(scanned);
                    break;
                }
                b'\r' if scanned + 1 < buf.len() => {
                    end = Some(scanned);
                    break;
                }
                b'\r' => break,
                _ => scanned += 1,
            }
        }
        if let Some(end) = end {
            buf.truncate(end);
            return String::from_utf8(buf).ok();
        }
        let n = match stream.read(&mut chunk) {
            Ok(n) => n,
            // A signal (or a tracer stop) interrupted the read: retry.
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        if n == 0 {
            // EOF: the rest is the line (a pending `\r` ends it).
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return String::from_utf8(buf).ok();
        }
        buf.extend_from_slice(&chunk[..n]);
        // Everything read so far is decoded: invalid UTF-8 fails the
        // request even past the line end; a split sequence may complete.
        if let Err(err) = std::str::from_utf8(&buf) {
            if err.error_len().is_some() {
                return None;
            }
        }
    }
}

fn handle(shared: &Shared, mut stream: UnixStream) {
    {
        let mut queue = lock_queue(&shared.queue);
        if queue.cleanup(now(), shared.signals.as_ref()).is_err() {
            return;
        }
    }
    let Some(line) = read_line(&mut stream) else {
        return;
    };
    let request = pystr::strip(&line);
    let mut parts = request.split(':');
    let command = parts.next().unwrap_or("");
    let arg = parts.next();
    let signals = shared.signals.as_ref();
    let id_arg = || arg.and_then(pystr::parse_int);
    let reply = match command {
        // Number of notifications, critical ones.
        "num" => {
            let queue = lock_queue(&shared.queue);
            Some(format!("{},{}", queue.len(), queue.critical_count()))
        }
        // Dismiss one notification.
        "del" => {
            let Some(id) = id_arg() else {
                return;
            };
            let _ = lock_queue(&shared.queue).remove(&id, DISMISSED, signals);
            None
        }
        // Dismiss a comma-separated list; one bad id cancels the request.
        "delm" => {
            let Some(arg) = arg else {
                return;
            };
            let Some(ids) = arg
                .split(',')
                .map(pystr::parse_int)
                .collect::<Option<Vec<BigInt>>>()
            else {
                return;
            };
            let _ = lock_queue(&shared.queue).remove_all(&ids, DISMISSED, signals);
            None
        }
        // Dismiss every notification of an application. The name is
        // everything after the first `:`, so names that contain a colon
        // ("Chat: Work") are matched whole.
        "dela" => {
            let Some((_, application)) = request.split_once(':') else {
                return;
            };
            let mut queue = lock_queue(&shared.queue);
            let ids = queue.ids_of_application(application);
            let _ = queue.remove_all(&ids, DISMISSED, signals);
            None
        }
        // The list document.
        "list" => Some(lock_queue(&shared.queue).list_json()),
        // Seen: urgency back to normal, the default action invoked.
        "see" => {
            let Some(id) = id_arg() else {
                return;
            };
            let _ = lock_queue(&shared.queue).see(&id, signals);
            None
        }
        _ => None,
    };
    if let Some(reply) = reply {
        let _ = stream.write_all(reply.as_bytes());
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}
