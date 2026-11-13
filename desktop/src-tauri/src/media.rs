//! Serves bytes from the phone to the webview as `orither://localhost/<feature>/<method>?b=<json>`,
//! so that a thumbnail is simply an `<img>`. Each answer is cached on disk,
//! because a gallery asks for the same few thousand pictures every time.

use std::{borrow::Cow, fs, path::PathBuf};

use anyhow::{Context, Result};
use orither_link::Duplex;
use serde_json::Value;
use tauri::{
    AppHandle, Manager, Runtime, UriSchemeContext, UriSchemeResponder, async_runtime,
    http::{Request, Response, StatusCode, header},
};

use crate::link::Link;

const CACHE_FOR_A_DAY: &str = "max-age=86400";

pub fn serve<R: Runtime>(context: UriSchemeContext<'_, R>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = context.app_handle().clone();
    let app: AppHandle<R> = app;
    async_runtime::spawn(async move {
        let response = match fetch(&app, &request).await {
            Ok(bytes) => respond(StatusCode::OK, bytes),
            Err(err) => {
                tracing::debug!("{}: {err:#}", request.uri());
                respond(StatusCode::NOT_FOUND, Vec::new())
            }
        };
        responder.respond(response);
    });
}

fn respond(status: StatusCode, bytes: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, CACHE_FOR_A_DAY)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(Cow::Owned(bytes))
        .expect("the headers are valid")
}

async fn fetch<R: Runtime>(app: &AppHandle<R>, request: &Request<Vec<u8>>) -> Result<Vec<u8>> {
    let uri = request.uri();
    let cached = cache_path(app, &uri.to_string())?;
    if let Ok(bytes) = fs::read(&cached) {
        return Ok(bytes);
    }

    let mut path = uri.path().trim_matches('/').split('/');
    let feature = path.next().context("no feature")?;
    let method = path.next().context("no method")?;
    let query = uri.query().context("no body")?;
    let param = |name: &str| query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='));
    let body: Value = serde_json::from_str(&decode(param("b").context("no body")?)?)?;
    let peer = param("p").map(decode).transpose()?;

    let link = app.state::<Link>();
    let session = link.session_for(peer.as_deref())?;
    let stream: Duplex = session.open(feature, method, body).await?;
    let mut recv = stream.recv;
    let bytes = recv.read_to_end(usize::MAX).await?;
    fs::write(&cached, &bytes).context("caching")?;
    Ok(bytes)
}

fn cache_path<R: Runtime>(app: &AppHandle<R>, uri: &str) -> Result<PathBuf> {
    let directory = app.path().app_cache_dir()?.join("media");
    fs::create_dir_all(&directory)?;
    Ok(directory.join(blake3::hash(uri.as_bytes()).to_hex().as_str()))
}

/// Undoes `encodeURIComponent`.
fn decode(text: &str) -> Result<String> {
    let bytes = text.as_bytes();
    let (mut decoded, mut index) = (Vec::with_capacity(bytes.len()), 0);
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3).context("cut-off escape")?;
            decoded.push(u8::from_str_radix(hex, 16)?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(String::from_utf8(decoded)?)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_what_the_ui_encodes() {
        assert_eq!(decode("%7B%22id%22%3A7%7D").unwrap(), r#"{"id":7}"#);
        assert_eq!(decode("%C3%A8").unwrap(), "è");
        assert!(decode("%2").is_err());
    }
}
