//! Socket client used by the viewer and the status block.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use crate::bigint::BigInt;
use crate::json::{self, Json};
use crate::pystr;
use crate::SOCKET_PATH;

/// Why a client operation failed.
#[derive(Debug)]
pub enum ClientError {
    /// The daemon is down: the socket file is missing or refuses
    /// connections (`FileNotFoundError` / `ConnectionRefusedError`). The
    /// status block reports this; the viewer dies on it.
    Down(String),
    /// Anything else: the binary dies with status 1.
    Fatal(String),
}

impl ClientError {
    pub fn from_io(context: &str, err: io::Error) -> ClientError {
        match err.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                ClientError::Down(format!("{context}: {err}"))
            }
            _ => ClientError::Fatal(format!("{context}: {err}")),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            ClientError::Down(m) | ClientError::Fatal(m) => m,
        }
    }
}

pub struct Client {
    path: String,
}

impl Default for Client {
    fn default() -> Self {
        Client::new(SOCKET_PATH)
    }
}

impl Client {
    pub fn new(path: &str) -> Client {
        Client {
            path: path.to_string(),
        }
    }

    fn connect(&self) -> Result<UnixStream, ClientError> {
        UnixStream::connect(&self.path).map_err(|e| ClientError::from_io(&self.path, e))
    }

    fn send(&self, command: &str, arg: &str) -> Result<(), ClientError> {
        let mut stream = self.connect()?;
        stream
            .write_all(format!("{command}:{arg}\n").as_bytes())
            .map_err(|e| ClientError::from_io("send", e))
    }

    /// `num`: (notifications, critical ones). One read of at most 32 bytes.
    pub fn count(&self) -> Result<(BigInt, BigInt), ClientError> {
        let mut stream = self.connect()?;
        stream
            .write_all(b"num\n")
            .map_err(|e| ClientError::from_io("send", e))?;
        let mut buf = [0u8; 32];
        let n = stream
            .read(&mut buf)
            .map_err(|e| ClientError::from_io("recv", e))?;
        let text = std::str::from_utf8(&buf[..n])
            .map_err(|_| ClientError::Fatal("reply is not UTF-8".into()))?;
        let parts: Vec<&str> = text.splitn(3, ',').collect();
        let parse = |s: &str| {
            pystr::parse_int(s).ok_or_else(|| ClientError::Fatal(format!("invalid count {s:?}")))
        };
        match parts.as_slice() {
            [num, crit] => Ok((parse(num)?, parse(crit)?)),
            _ => Err(ClientError::Fatal(format!("invalid count reply {text:?}"))),
        }
    }

    pub fn delete(&self, id: &str) -> Result<(), ClientError> {
        self.send("del", id)
    }

    pub fn delete_all(&self, application: &str) -> Result<(), ClientError> {
        self.send("dela", application)
    }

    pub fn see(&self, id: &str) -> Result<(), ClientError> {
        self.send("see", id)
    }

    /// `list`: the whole reply, read to EOF, as JSON.
    pub fn list(&self) -> Result<Json, ClientError> {
        let mut stream = self.connect()?;
        stream
            .write_all(b"list\n")
            .map_err(|e| ClientError::from_io("send", e))?;
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .map_err(|e| ClientError::from_io("recv", e))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| ClientError::Fatal("list reply is not UTF-8".into()))?;
        json::parse(&text).map_err(|_| ClientError::Fatal("list reply is not JSON".into()))
    }
}
