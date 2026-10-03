//! Reading a response body with a cap that holds after decompression.

use std::io::Read;

use ureq::Body;
use ureq::http::Response;

/// Reads a body as text, refusing more than `limit` bytes.
///
/// ureq's own limit counts bytes on the wire, so a small gzip body could
/// inflate without bound; the decoded stream is therefore capped here as well.
/// Declared charsets are decoded and invalid UTF-8 is replaced.
///
/// # Errors
/// Whatever ureq reports while reading, or [`ureq::Error::BodyExceedsLimit`].
pub fn read_text(response: &mut Response<Body>, limit: u64) -> Result<String, ureq::Error> {
    let mut bytes = Vec::new();
    let _: usize = response
        .body_mut()
        .with_config()
        .limit(limit)
        .lossy_utf8(true)
        .reader()
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(ureq::Error::from)?;
    if u64::try_from(bytes.len()).is_ok_and(|read| read <= limit) {
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        Err(ureq::Error::BodyExceedsLimit(limit))
    }
}
