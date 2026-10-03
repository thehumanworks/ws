//! One isolated, bounded local browser process per operation.

use std::fmt;
use std::io::Read;
use std::net::Ipv6Addr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

use crate::browser_asset;
use crate::config::Env;
use crate::error::Error;
use crate::fetch::{Access, ContentKind, MAX_PAGE_BYTES, Page};
use crate::output;

const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const V4_BLOCKS: [([u8; 4], u8); 11] = [
    ([0, 0, 0, 0], 8),
    ([10, 0, 0, 0], 8),
    ([100, 64, 0, 0], 10),
    ([127, 0, 0, 0], 8),
    ([169, 254, 0, 0], 16),
    ([172, 16, 0, 0], 12),
    ([192, 0, 0, 0], 24),
    ([192, 168, 0, 0], 16),
    ([198, 18, 0, 0], 15),
    ([224, 0, 0, 0], 3),
    ([240, 0, 0, 0], 4),
];

/// Browser dump requested; ws performs HTML selection/conversion itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dump {
    /// Serialized DOM after JavaScript execution, with nothing stripped.
    Html,
    /// Lightpanda's whole-page text-only PNG rendering.
    Png,
}

impl Dump {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Png => "png",
        }
    }
}

/// One browser operation; credentials are never part of this interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Original validated HTTP(S) URL.
    pub url: String,
    /// Requested output.
    pub dump: Dump,
    /// Whether private addresses may be reached.
    pub access: Access,
    /// Wall deadline including every navigation and dump.
    pub timeout: Duration,
}

/// Decoded local output, ready for the shared page pipeline or binary writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    /// Unmodified rendered DOM and request/response metadata.
    Html(Page),
    /// Decoded PNG bytes, never terminal-sanitized.
    Png(Vec<u8>),
}

/// Browser boundary injectable in CLI tests; the production implementation
/// always executes the pinned embedded asset.
pub trait Browser {
    /// Performs one operation, with no retries or fallback.
    ///
    /// # Errors
    /// Configuration, process, network, status or decoding failures.
    fn retrieve(&self, request: &Request) -> Result<Rendered, Error>;
}

/// The bundled browser, with an injected environment used only for its cache.
pub struct Bundled<'a> {
    env: Env<'a>,
}

impl fmt::Debug for Bundled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundled").finish_non_exhaustive()
    }
}

impl<'a> Bundled<'a> {
    /// Creates a lazy runtime: extraction occurs only when retrieving a page.
    #[must_use]
    pub const fn new(env: Env<'a>) -> Self {
        Self { env }
    }
}

impl Browser for Bundled<'_> {
    fn retrieve(&self, request: &Request) -> Result<Rendered, Error> {
        // Internally encoded search URLs may exceed the user fetch URL limit.
        if !http_url(&request.url) {
            return Err(Error::Config(
                "browser URL must be absolute HTTP(S)".to_owned(),
            ));
        }
        let path = browser_asset::install(self.env)?;
        let mut command = browser_command(&path, request);
        let (success, bytes) = capture(&mut command, request.timeout, MAX_CAPTURE_BYTES)?;
        decode(&bytes, request, success)
    }
}

fn blocked_cidrs() -> Vec<String> {
    let mut cidrs = vec!["ff00::/8".to_owned()];
    for (octets, prefix) in V4_BLOCKS {
        let [a, b, c, d] = octets;
        cidrs.push(format!("{a}.{b}.{c}.{d}/{prefix}"));
        // Upstream normalizes ::ffff mapped addresses to IPv4 at the socket
        // boundary, but does not normalize compatible or well-known NAT64.
        let (high, low) = (u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d]));
        for address in [
            Ipv6Addr::new(0, 0, 0, 0, 0, 0, high, low),
            Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, high, low),
        ] {
            cidrs.push(format!("{address}/{}", 96_u8.saturating_add(prefix)));
        }
    }
    cidrs
}

fn browser_command(path: &Path, request: &Request) -> Command {
    let millis = request.timeout.as_millis().to_string();
    let mut command = Command::new(path);
    let _ = command
        .env_clear()
        .env("LIGHTPANDA_DISABLE_TELEMETRY", "true")
        .env("LIGHTPANDA_DISABLE_CORE_DUMP", "true")
        .env("DO_NOT_TRACK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "fetch",
            &request.url,
            "--dump",
            request.dump.as_str(),
            "--json",
            "--wait-until",
            "networkalmostidle",
            "--wait-ms",
            &millis,
            "--terminate-ms",
            &millis,
            "--http-timeout",
            &millis,
            "--http-connect-timeout",
            &millis,
            "--watchdog-ms",
            &millis,
            "--http-max-response-size",
            &MAX_PAGE_BYTES.to_string(),
            "--v8-max-heap-mb",
            "256",
            "--load-resources",
            "stylesheet",
            "--load-resources",
            "iframe",
            "--log-level",
            "fatal",
        ]);
    if request.dump == Dump::Html {
        let _ = command.args(["--dump-max-bytes", &MAX_PAGE_BYTES.to_string()]);
    }
    if request.access == Access::PublicOnly {
        let _ = command.arg("--block-private-networks");
        for cidr in blocked_cidrs() {
            let _ = command.args(["--block-cidrs", &cidr]);
        }
    }
    command
}

fn bounded_read(input: impl Read, limit: usize) -> Result<Vec<u8>, Error> {
    let cap = u64::try_from(limit)
        .map_err(|_| Error::Transport("invalid browser output limit".to_owned()))?
        .saturating_add(1);
    let mut bytes = Vec::new();
    let _ = input.take(cap).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Transport(
            "Lightpanda exceeded its captured-output limit".to_owned(),
        ));
    }
    Ok(bytes)
}

fn capture(
    command: &mut Command,
    timeout: Duration,
    limit: usize,
) -> Result<(bool, Vec<u8>), Error> {
    let start = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        Error::Transport(format!("could not start bundled Lightpanda: {error}"))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Transport("missing browser stdout".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Transport("missing browser stderr".to_owned()))?;
    let (sender, receiver) = mpsc::channel();
    let errors = sender.clone();
    let _ = thread::spawn(move || {
        let _ = sender.send((true, bounded_read(stdout, limit)));
    });
    let _ = thread::spawn(move || {
        let _ = errors.send((false, bounded_read(stderr, MAX_STDERR_BYTES)));
    });
    let mut output = None;
    let mut stderr_done = false;
    let result = loop {
        if start.elapsed() >= timeout {
            break Err(Error::Transport(
                "Lightpanda exceeded the wall timeout; increase --timeout if needed".to_owned(),
            ));
        }
        match receiver.recv_timeout(Duration::from_millis(10)) {
            Ok((is_stdout, Ok(bytes))) => {
                if is_stdout {
                    output = Some(bytes);
                } else {
                    stderr_done = true;
                }
            }
            Ok((_, Err(error))) => break Err(error),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) if stderr_done && output.is_some() => {
                break output.take().map_or_else(
                    || Err(Error::Transport("missing browser output".to_owned())),
                    |bytes| Ok((status.success(), bytes)),
                );
            }
            Ok(Some(_) | None) => {}
            Err(error) => break Err(error.into()),
        }
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

#[derive(Debug, Deserialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct Response {
    url: String,
    http_status: Option<u16>,
    #[serde(default)]
    headers: Vec<Header>,
    dump: Option<String>,
    content: Option<String>,
    error: Option<String>,
}

fn decode(bytes: &[u8], request: &Request, success: bool) -> Result<Rendered, Error> {
    if bytes.is_empty() && !success {
        return Err(Error::Transport(
            "Lightpanda failed before producing a response".to_owned(),
        ));
    }
    let response: Response = serde_json::from_slice(bytes)
        .map_err(|_| Error::UnexpectedResponse("Lightpanda returned malformed JSON".to_owned()))?;
    if let Some(error) = response.error {
        return Err(Error::Transport(match error.as_str() {
            "BotChallenge" => "Lightpanda encountered a bot challenge; access was denied and ws will not retry or bypass it".to_owned(),
            "BlockedIP" | "BlockedIp" | "IpBlocked" => "Lightpanda blocked a non-public address; pass --allow-private only for a trusted local page".to_owned(),
            _ => format!("Lightpanda navigation/dump failed: {}", output::sanitize(&error)),
        }));
    }
    if !success {
        return Err(Error::Transport("Lightpanda process failed".to_owned()));
    }
    if !http_url(&response.url) {
        return Err(Error::UnexpectedResponse(
            "Lightpanda returned an invalid final URL".to_owned(),
        ));
    }
    let status = response
        .http_status
        .ok_or_else(|| Error::UnexpectedResponse("Lightpanda omitted HTTP status".to_owned()))?;
    if !(200..300).contains(&status) {
        return Err(Error::PageStatus {
            status,
            url: output::sanitize(&response.url),
        });
    }
    if response.dump.as_deref() != Some(request.dump.as_str()) {
        return Err(Error::UnexpectedResponse(
            "Lightpanda returned the wrong dump format".to_owned(),
        ));
    }
    let content = response
        .content
        .ok_or_else(|| Error::UnexpectedResponse("Lightpanda omitted dumped content".to_owned()))?;
    match request.dump {
        Dump::Html => {
            if u64::try_from(content.len()).unwrap_or(u64::MAX) > MAX_PAGE_BYTES
                || content.trim_end().ends_with("[truncated]")
            {
                return Err(Error::PageContent(
                    "rendered DOM exceeds the page limit".to_owned(),
                ));
            }
            let content_type = response
                .headers
                .into_iter()
                .find(|header| header.name.eq_ignore_ascii_case("content-type"))
                .map(|header| header.value);
            Ok(Rendered::Html(Page {
                url: request.url.clone(),
                final_url: response.url,
                status,
                content_type,
                kind: ContentKind::Html,
                body: content,
            }))
        }
        Dump::Png => {
            if u64::try_from(content.len()).unwrap_or(u64::MAX) > MAX_PAGE_BYTES.saturating_mul(2) {
                return Err(Error::PageContent("PNG exceeds the page limit".to_owned()));
            }
            let png = STANDARD.decode(content).map_err(|_| {
                Error::UnexpectedResponse("Lightpanda returned invalid PNG base64".to_owned())
            })?;
            if u64::try_from(png.len()).unwrap_or(u64::MAX) > MAX_PAGE_BYTES
                || !png.starts_with(b"\x89PNG\r\n\x1a\n")
                || !png.ends_with(b"\x00\x00\x00\x00IEND\xaeB`\x82")
            {
                return Err(Error::UnexpectedResponse(
                    "Lightpanda returned invalid or oversized PNG data".to_owned(),
                ));
            }
            validate_png(&png)?;
            Ok(Rendered::Png(png))
        }
    }
}

fn validate_png(bytes: &[u8]) -> Result<(), Error> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: 128 * 1024 * 1024,
    });
    let invalid =
        || Error::UnexpectedResponse("Lightpanda returned an invalid PNG image".to_owned());
    let mut reader = decoder.read_info().map_err(|_| invalid())?;
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= 128 * 1024 * 1024)
        .ok_or_else(|| {
            Error::PageContent("decoded PNG exceeds the pixel buffer limit".to_owned())
        })?;
    let _ = reader
        .next_frame(&mut vec![0; size])
        .map_err(|_| invalid())?;
    Ok(())
}

fn http_url(url: &str) -> bool {
    !url.chars()
        .any(|character| character.is_whitespace() || character.is_control())
        && url.parse::<ureq::http::Uri>().is_ok_and(|uri| {
            matches!(uri.scheme_str(), Some("http" | "https")) && uri.host().is_some()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(dump: Dump) -> Request {
        Request {
            url: "https://example.com/".to_owned(),
            dump,
            access: Access::PublicOnly,
            timeout: Duration::from_secs(3),
        }
    }

    fn response(content: &str, status: u16, error: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&json!({ "url": "https://final.example/", "http_status": status, "headers": [{"name":"Content-Type", "value":"text/html"}], "dump":"html", "content":content, "error":error })).unwrap()
    }

    #[test]
    fn parses_status_and_final_url_without_losing_original() {
        let Rendered::Html(page) = decode(
            &response("<main>JS</main>", 200, None),
            &request(Dump::Html),
            true,
        )
        .unwrap() else {
            panic!("wrong output");
        };
        assert_eq!(page.url, "https://example.com/");
        assert_eq!(page.final_url, "https://final.example/");
        assert_eq!(page.content_type.as_deref(), Some("text/html"));
        assert!(
            decode(&response("denied", 403, None), &request(Dump::Html), true)
                .unwrap_err()
                .to_string()
                .contains("HTTP 403")
        );
        assert!(
            decode(
                &response("challenge", 200, Some("BotChallenge")),
                &request(Dump::Html),
                false
            )
            .unwrap_err()
            .to_string()
            .contains("will not retry or bypass")
        );
        assert!(decode(b"garbage", &request(Dump::Html), false).is_err());
        assert!(
            decode(
                &response("[truncated]", 200, None),
                &request(Dump::Html),
                true
            )
            .is_err()
        );
    }

    #[test]
    fn binary_png_is_decoded_without_text_processing() {
        let png = include_bytes!("../tests/fixtures/tiny.png");
        let bytes = serde_json::to_vec(&json!({"url":"https://example.com/", "http_status":200,"dump":"png","content":STANDARD.encode(png),"error":null})).unwrap();
        assert_eq!(
            decode(&bytes, &request(Dump::Png), true).unwrap(),
            Rendered::Png(png.to_vec())
        );
        assert!(decode(&bytes, &request(Dump::Html), true).is_err());
    }

    #[test]
    fn process_environment_and_all_network_limits_are_explicit() {
        let command = browser_command(Path::new("lightpanda"), &request(Dump::Html));
        assert_eq!(command.get_envs().count(), 3);
        let args = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        for required in [
            "--block-private-networks",
            "100.64.0.0/10",
            "192.0.0.0/24",
            "198.18.0.0/15",
            "224.0.0.0/3",
            "ff00::/8",
            "64:ff9b::a00:0/104",
            "::a00:0/104",
            "--http-max-response-size",
            "--terminate-ms",
        ] {
            assert!(args.iter().any(|arg| arg == required), "missing {required}");
        }
        let mut private = request(Dump::Html);
        private.access = Access::AllowPrivate;
        assert!(
            !browser_command(Path::new("lightpanda"), &private)
                .get_args()
                .any(|argument| argument == "--block-private-networks")
        );
        let long = crate::brave::search_url(&"🦀".repeat(1024));
        assert!(http_url(&long));
    }

    #[cfg(unix)]
    #[test]
    fn child_capture_enforces_deadline_and_output_limit() {
        let mut timeout = Command::new("/bin/sleep");
        let _ = timeout
            .arg("5")
            .env_clear()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let start = Instant::now();
        assert!(
            capture(&mut timeout, Duration::from_millis(100), 100)
                .unwrap_err()
                .to_string()
                .contains("wall timeout")
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        let mut overflow = Command::new("/usr/bin/yes");
        let _ = overflow
            .env_clear()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        assert!(
            capture(&mut overflow, Duration::from_secs(2), 100)
                .unwrap_err()
                .to_string()
                .contains("captured-output limit")
        );
        assert!(bounded_read(&b"12345"[..], 4).is_err());
    }

    #[test]
    fn native_network_filter_covers_redirects_and_script_iframe_fetch_xhr_websocket() {
        use std::io::Write;
        use std::net::TcpListener;
        if crate::backend::Backend::platform_default() != crate::backend::Backend::Lightpanda {
            return;
        }
        let cache = tempfile::tempdir().unwrap();
        let env = |key: &str| (key == "WS_CACHE_DIR").then(|| cache.path().display().to_string());
        let executable = browser_asset::install(&env).unwrap();
        // This test-only bootstrap exemption allows a local origin while every
        // target at IPv6 loopback still goes through the production IP filter.
        let denied = TcpListener::bind("[::1]:0").unwrap();
        denied.set_nonblocking(true).unwrap();
        let denied_url = format!("http://[::1]:{}", denied.local_addr().unwrap().port());
        let ws_url = format!("ws://[::1]:{}", denied.local_addr().unwrap().port());
        let html = format!(
            "<html><head><link rel='stylesheet' href='{denied_url}/css'></head><body><main>Filtered page</main><iframe src='{denied_url}/frame'></iframe><script src='{denied_url}/script'></script><script>fetch('{denied_url}/fetch').catch(()=>{{}});let x=new XMLHttpRequest();x.open('GET','{denied_url}/xhr');x.send();let w=new WebSocket('{ws_url}/ws');</script></body></html>"
        );
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", origin.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = origin.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).unwrap();
        });
        let mut operation = request(Dump::Html);
        operation.url = url;
        let mut command = browser_command(&executable, &operation);
        let _ = command.args(["--block-cidrs", "-127.0.0.1/32"]);
        let (success, bytes) = capture(&mut command, operation.timeout, MAX_CAPTURE_BYTES).unwrap();
        assert!(decode(&bytes, &operation, success).is_ok());
        server.join().unwrap();
        assert_eq!(
            denied.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let redirect_origin = TcpListener::bind("127.0.0.1:0").unwrap();
        operation.url = format!("http://{}", redirect_origin.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = redirect_origin.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 302 Found\r\nLocation: {denied_url}/redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let mut command = browser_command(&executable, &operation);
        let _ = command.args(["--block-cidrs", "-127.0.0.1/32"]);
        let (success, bytes) = capture(&mut command, operation.timeout, MAX_CAPTURE_BYTES).unwrap();
        assert!(decode(&bytes, &operation, success).is_err());
        server.join().unwrap();
        assert_eq!(
            denied.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
