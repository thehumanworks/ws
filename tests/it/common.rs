//! A tiny single-threaded HTTP/1.1 mock server for integration tests.
//!
//! It answers each incoming connection with the next canned response and
//! records what the client sent, so tests can assert on the exact wire format.

use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

/// A response the mock server will send.
#[derive(Debug, Clone)]
pub struct Canned {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub delay: Duration,
}

impl Canned {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: body.as_bytes().to_vec(),
            delay: Duration::ZERO,
        }
    }

    /// A response with the given `Content-Type` (none when empty) and raw body.
    pub fn page(status: u16, content_type: &str, body: &[u8]) -> Self {
        let headers = if content_type.is_empty() {
            Vec::new()
        } else {
            vec![("Content-Type".to_owned(), content_type.to_owned())]
        };
        Self {
            status,
            headers,
            body: body.to_vec(),
            delay: Duration::ZERO,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub const fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// A request the mock server received.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub struct MockServer {
    pub url: String,
    requests: Receiver<Recorded>,
}

impl MockServer {
    /// Starts a server that answers one connection per canned response, in order.
    pub fn start(responses: Vec<Canned>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let url = format!(
            "http://127.0.0.1:{}",
            listener.local_addr().expect("local addr").port()
        );
        let (sender, requests) = channel();
        let _detached = thread::spawn(move || {
            for canned in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    return;
                }
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let path = parts.next().unwrap_or_default().to_owned();
                let mut headers = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        headers.push((name.trim().to_owned(), value.trim().to_owned()));
                    }
                }
                let length = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let body = String::from_utf8_lossy(&body).into_owned();
                if sender
                    .send(Recorded {
                        method,
                        path,
                        headers,
                        body,
                    })
                    .is_err()
                {
                    return;
                }
                thread::sleep(canned.delay);
                let mut head = format!("HTTP/1.1 {} Mock\r\n", canned.status);
                for (name, value) in &canned.headers {
                    let _ = write!(head, "{name}: {value}\r\n");
                }
                let _ = write!(
                    head,
                    "Content-Length: {}\r\nConnection: close\r\n\r\n",
                    canned.body.len()
                );
                // The client may have given up already (timeout tests); ignore write errors.
                drop(stream.write_all(head.as_bytes()));
                drop(stream.write_all(&canned.body));
                drop(stream.flush());
            }
        });
        Self { url, requests }
    }

    /// Every request received so far.
    pub fn received(&self) -> Vec<Recorded> {
        self.requests.try_iter().collect()
    }
}
