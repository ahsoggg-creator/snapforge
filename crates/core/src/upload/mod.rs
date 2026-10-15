//! Uploading a screenshot. OFF by default: nothing is sent until the user switches uploads on, enables a destination
//! and configures it. Tokens and keys come from the Credential Manager (`secrets`), never from the settings file.
//!
//! Destinations: a custom HTTP endpoint, a Discord webhook, a Telegram bot, an S3-compatible bucket.

pub mod sigv4;

use serde::{Deserialize, Serialize};

use crate::diag;
use crate::secrets::{self, SecretStore};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpDest {
    pub url: String,
    /// `POST` or `PUT`.
    pub method: String,
    /// Header values may contain `{secret:name}`, filled from the Credential Manager.
    pub headers: Vec<(String, String)>,
    /// Multipart field that carries the file; empty = send the raw bytes as the body.
    pub file_field: String,
    /// Extra multipart fields (values may contain `{secret:name}`).
    pub fields: Vec<(String, String)>,
    /// Where the link is in a JSON response, e.g. `data.link` or `files[0].url`; empty = the body is the link.
    pub json_path: String,
}

impl Default for HttpDest {
    fn default() -> HttpDest {
        HttpDest {
            url: String::new(),
            method: "POST".into(),
            headers: Vec::new(),
            file_field: "file".into(),
            fields: Vec::new(),
            json_path: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct S3Dest {
    /// `https://s3.eu-central-1.amazonaws.com`, a MinIO / R2 / Backblaze endpoint, …
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    /// Key prefix, e.g. `screenshots/`.
    pub prefix: String,
    /// If set, the returned link is `public_base/key` (a CDN or a public bucket URL).
    pub public_base: String,
    /// `endpoint/bucket/key` (true) or `bucket.endpoint/key` (false).
    pub path_style: bool,
}

impl Default for S3Dest {
    fn default() -> S3Dest {
        S3Dest {
            endpoint: String::new(),
            bucket: String::new(),
            region: "us-east-1".into(),
            prefix: String::new(),
            public_base: String::new(),
            path_style: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DestKind {
    Http(HttpDest),
    /// The webhook URL is the secret `<id>.webhook`.
    Discord,
    /// The bot token is the secret `<id>.token`; `chat_id` is a number or `@channel`.
    Telegram {
        chat_id: String,
    },
    /// Secrets `<id>.access_key` and `<id>.secret_key`.
    S3(S3Dest),
}

impl DestKind {
    pub fn label(&self) -> &'static str {
        match self {
            DestKind::Http(_) => "HTTP",
            DestKind::Discord => "Discord",
            DestKind::Telegram { .. } => "Telegram",
            DestKind::S3(_) => "S3",
        }
    }

    /// Names (without the `<id>.` prefix) of the secrets this kind needs.
    pub fn secret_names(&self) -> &'static [&'static str] {
        match self {
            DestKind::Http(_) => &["token"],
            DestKind::Discord => &["webhook"],
            DestKind::Telegram { .. } => &["token"],
            DestKind::S3(_) => &["access_key", "secret_key"],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub name: String,
    /// Off until the user turns it on.
    #[serde(default)]
    pub enabled: bool,
    pub kind: DestKind,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UploadSettings {
    /// Master switch. Off by default; with it off nothing can be uploaded whatever else is configured.
    pub enabled: bool,
    pub destinations: Vec<Destination>,
    /// Destination used by the "Upload" action.
    pub active: Option<String>,
}

pub struct FilePart {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait Transport: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String>;
}

/// Real network transport (Windows' own TLS stack).
pub struct UreqTransport;

impl Transport for UreqTransport {
    fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
        let mut b = ureq::http::Request::builder().method(r.method.as_str()).uri(&r.url);
        b = b.header("User-Agent", crate::net::user_agent());
        for (k, v) in &r.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        let req = b.body(r.body.clone()).map_err(|e| e.to_string())?;
        let mut resp = crate::net::agent(120)
            .run(req)
            .map_err(|e| diag::redact(&e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, body })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("uploads are switched off (Settings → Uploads); nothing was sent")]
    Disabled,
    #[error("no upload destination is selected")]
    NoDestination,
    #[error("the destination '{0}' is switched off; nothing was sent")]
    DestinationOff(String),
    #[error("the destination is not fully configured: {0}")]
    Config(String),
    #[error("{0}")]
    Secret(String),
    #[error("network error: {0}")]
    Transport(String),
    #[error("the server answered HTTP {status}: {message}")]
    Server { status: u16, message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct UploadOutcome {
    /// Link to copy to the clipboard, if the destination gives one.
    pub link: Option<String>,
    pub note: String,
}

/// A store that prefixes every name with `<id>.` so a destination only reaches its own secrets.
struct Scoped<'a> {
    inner: &'a dyn SecretStore,
    id: &'a str,
}

impl SecretStore for Scoped<'_> {
    fn get(&self, name: &str) -> Option<String> {
        self.inner.get(&format!("{}.{name}", self.id))
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.inner.set(&format!("{}.{name}", self.id), value)
    }

    fn delete(&self, name: &str) {
        self.inner.delete(&format!("{}.{name}", self.id));
    }
}

fn boundary() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("----SnapForge{:032x}", n ^ 0x5eed_5eed_5eed)
}

/// Builds a `multipart/form-data` body: text fields first, then the file.
pub fn multipart(boundary: &str, fields: &[(String, String)], file_field: &str, file: &FilePart) -> Vec<u8> {
    let mut b = Vec::new();
    for (k, v) in fields {
        b.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes(),
        );
    }
    b.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
            file.name.replace('"', "_"),
            file.mime
        )
        .as_bytes(),
    );
    b.extend_from_slice(&file.bytes);
    b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    b
}

/// Reads a value from JSON by a path like `data.link`, `files[0].url` or `a.b[2].c`.
pub fn json_path(json: &str, path: &str) -> Option<String> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    let mut v = &root;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        let (name, idx) = match seg.split_once('[') {
            Some((n, rest)) => (n, Some(rest.trim_end_matches(']').parse::<usize>().ok()?)),
            None => (seg, None),
        };
        if !name.is_empty() {
            v = v.get(name)?;
        }
        if let Some(i) = idx {
            v = v.get(i)?;
        }
    }
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn check_status(resp: &HttpResponse) -> Result<(), UploadError> {
    if (200..300).contains(&resp.status) {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&resp.body);
    let message: String = diag::redact(text.trim()).chars().take(200).collect();
    Err(UploadError::Server {
        status: resp.status,
        message,
    })
}

fn send(t: &dyn Transport, req: HttpRequest) -> Result<HttpResponse, UploadError> {
    let resp = t.send(&req).map_err(UploadError::Transport)?;
    check_status(&resp)?;
    Ok(resp)
}

fn upload_http(d: &HttpDest, scope: &Scoped, file: &FilePart, t: &dyn Transport) -> Result<UploadOutcome, UploadError> {
    if d.url.trim().is_empty() {
        return Err(UploadError::Config("the URL is empty".into()));
    }
    let method = d.method.to_ascii_uppercase();
    if method != "POST" && method != "PUT" {
        return Err(UploadError::Config("the method must be POST or PUT".into()));
    }
    let ex = |s: &str| secrets::expand(s, scope).map_err(UploadError::Secret);
    let url = ex(&d.url)?;
    let mut headers = Vec::new();
    for (k, v) in &d.headers {
        headers.push((k.clone(), ex(v)?));
    }
    let (body, content_type) = if d.file_field.trim().is_empty() {
        (file.bytes.clone(), file.mime.clone())
    } else {
        let mut fields = Vec::new();
        for (k, v) in &d.fields {
            fields.push((k.clone(), ex(v)?));
        }
        let b = boundary();
        (
            multipart(&b, &fields, &d.file_field, file),
            format!("multipart/form-data; boundary={b}"),
        )
    };
    headers.push(("Content-Type".into(), content_type));
    let resp = send(
        t,
        HttpRequest {
            method,
            url,
            headers,
            body,
        },
    )?;
    let text = String::from_utf8_lossy(&resp.body).trim().to_owned();
    let link = if d.json_path.trim().is_empty() {
        text.starts_with("http").then(|| text.clone())
    } else {
        json_path(&text, &d.json_path)
    };
    let note = if link.is_some() {
        "uploaded".to_owned()
    } else {
        "uploaded, but no link was found in the response".to_owned()
    };
    Ok(UploadOutcome { link, note })
}

fn upload_discord(scope: &Scoped, file: &FilePart, t: &dyn Transport) -> Result<UploadOutcome, UploadError> {
    let hook = scope
        .get("webhook")
        .ok_or_else(|| UploadError::Secret("the Discord webhook URL is not set".into()))?;
    if !hook.starts_with("https://") {
        return Err(UploadError::Config(
            "the Discord webhook must be an https:// URL".into(),
        ));
    }
    let url = format!("{hook}{}wait=true", if hook.contains('?') { "&" } else { "?" });
    let b = boundary();
    let body = multipart(
        &b,
        &[("payload_json".into(), "{\"content\":\"\"}".into())],
        "files[0]",
        file,
    );
    let resp = send(
        t,
        HttpRequest {
            method: "POST".into(),
            url,
            headers: vec![("Content-Type".into(), format!("multipart/form-data; boundary={b}"))],
            body,
        },
    )?;
    let link = json_path(&String::from_utf8_lossy(&resp.body), "attachments[0].url");
    Ok(UploadOutcome {
        link,
        note: "sent to Discord".into(),
    })
}

fn upload_telegram(
    chat_id: &str,
    scope: &Scoped,
    file: &FilePart,
    t: &dyn Transport,
) -> Result<UploadOutcome, UploadError> {
    if chat_id.trim().is_empty() {
        return Err(UploadError::Config("the chat id is empty".into()));
    }
    let token = scope
        .get("token")
        .ok_or_else(|| UploadError::Secret("the Telegram bot token is not set".into()))?;
    let url = format!("https://api.telegram.org/bot{token}/sendDocument");
    let b = boundary();
    let body = multipart(&b, &[("chat_id".into(), chat_id.trim().into())], "document", file);
    let resp = send(
        t,
        HttpRequest {
            method: "POST".into(),
            url,
            headers: vec![("Content-Type".into(), format!("multipart/form-data; boundary={b}"))],
            body,
        },
    )?;
    let text = String::from_utf8_lossy(&resp.body).into_owned();
    // a message link exists only for chats with a public username
    let link = match (
        json_path(&text, "result.chat.username"),
        json_path(&text, "result.message_id"),
    ) {
        (Some(u), Some(id)) => Some(format!("https://t.me/{u}/{id}")),
        _ => None,
    };
    Ok(UploadOutcome {
        link,
        note: "sent to Telegram".into(),
    })
}

/// Current UTC time as `YYYYMMDDTHHMMSSZ`.
fn amz_now() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

fn s3_request(
    d: &S3Dest,
    scope: &Scoped,
    file: &FilePart,
    amz_date: &str,
) -> Result<(HttpRequest, Option<String>), UploadError> {
    if d.endpoint.trim().is_empty() || d.bucket.trim().is_empty() {
        return Err(UploadError::Config("the endpoint and the bucket are required".into()));
    }
    let access = scope
        .get("access_key")
        .ok_or_else(|| UploadError::Secret("the S3 access key is not set".into()))?;
    let secret = scope
        .get("secret_key")
        .ok_or_else(|| UploadError::Secret("the S3 secret key is not set".into()))?;
    let endpoint = d.endpoint.trim().trim_end_matches('/');
    let (scheme, host_part) = endpoint
        .split_once("://")
        .ok_or_else(|| UploadError::Config("the endpoint must start with https://".into()))?;
    let key = format!("{}{}", d.prefix.trim_start_matches('/'), file.name);
    let (host, path) = if d.path_style {
        (host_part.to_owned(), format!("/{}/{}", d.bucket, key))
    } else {
        (format!("{}.{host_part}", d.bucket), format!("/{key}"))
    };
    let canonical_uri = sigv4::uri_encode(&path, true);
    let payload = sigv4::sha256_hex(&file.bytes);
    let headers = vec![
        ("content-type".to_owned(), file.mime.clone()),
        ("host".to_owned(), host.clone()),
        ("x-amz-content-sha256".to_owned(), payload.clone()),
        ("x-amz-date".to_owned(), amz_date.to_owned()),
    ];
    let sig = sigv4::sign(&sigv4::SignInput {
        method: "PUT",
        canonical_uri: &canonical_uri,
        canonical_query: "",
        headers: &headers,
        payload_sha256: &payload,
        access_key: &access,
        secret_key: &secret,
        region: &d.region,
        service: "s3",
        amz_date,
    });
    let mut out_headers: Vec<(String, String)> = headers.into_iter().filter(|(k, _)| k != "host").collect();
    out_headers.push(("Authorization".into(), sig.authorization));
    let url = format!("{scheme}://{host}{canonical_uri}");
    let link = if d.public_base.trim().is_empty() {
        Some(url.clone())
    } else {
        Some(format!(
            "{}/{}",
            d.public_base.trim().trim_end_matches('/'),
            sigv4::uri_encode(&key, true)
        ))
    };
    Ok((
        HttpRequest {
            method: "PUT".into(),
            url,
            headers: out_headers,
            body: file.bytes.clone(),
        },
        link,
    ))
}

fn upload_s3(d: &S3Dest, scope: &Scoped, file: &FilePart, t: &dyn Transport) -> Result<UploadOutcome, UploadError> {
    let (req, link) = s3_request(d, scope, file, &amz_now())?;
    send(t, req)?;
    Ok(UploadOutcome {
        link,
        note: "uploaded to S3".into(),
    })
}

/// Uploads `file` to the destination `dest_id` (or the active one). Refuses — without touching the network — unless
/// uploads are switched on globally AND for that destination.
pub fn upload(
    s: &UploadSettings,
    dest_id: Option<&str>,
    file: &FilePart,
    store: &dyn SecretStore,
    t: &dyn Transport,
) -> Result<UploadOutcome, UploadError> {
    if !s.enabled {
        return Err(UploadError::Disabled);
    }
    let id = dest_id.or(s.active.as_deref()).ok_or(UploadError::NoDestination)?;
    let dest = s
        .destinations
        .iter()
        .find(|d| d.id == id)
        .ok_or(UploadError::NoDestination)?;
    if !dest.enabled {
        return Err(UploadError::DestinationOff(dest.name.clone()));
    }
    let scope = Scoped {
        inner: store,
        id: &dest.id,
    };
    let result = match &dest.kind {
        DestKind::Http(h) => upload_http(h, &scope, file, t),
        DestKind::Discord => upload_discord(&scope, file, t),
        DestKind::Telegram { chat_id } => upload_telegram(chat_id, &scope, file, t),
        DestKind::S3(d) => upload_s3(d, &scope, file, t),
    };
    // metadata only: never the URL, the headers or the response body
    match &result {
        Ok(_) => diag::log(
            diag::Level::Info,
            &format!("upload to {} ok ({} bytes)", dest.kind.label(), file.bytes.len()),
        ),
        Err(e) => diag::log(
            diag::Level::Warn,
            &format!("upload to {} failed: {e}", dest.kind.label()),
        ),
    }
    result
}

pub fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::TEST_SERIAL;
    use crate::secrets::MemoryStore;
    use std::sync::Mutex;

    /// Records requests and answers with a canned response.
    struct Mock {
        seen: Mutex<Vec<HttpRequest>>,
        reply: HttpResponse,
    }

    impl Mock {
        fn new(status: u16, body: &str) -> Mock {
            Mock {
                seen: Mutex::new(Vec::new()),
                reply: HttpResponse {
                    status,
                    body: body.as_bytes().to_vec(),
                },
            }
        }

        fn requests(&self) -> Vec<HttpRequest> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for Mock {
        fn send(&self, r: &HttpRequest) -> Result<HttpResponse, String> {
            self.seen.lock().unwrap().push(r.clone());
            Ok(self.reply.clone())
        }
    }

    fn file() -> FilePart {
        FilePart {
            name: "shot.png".into(),
            mime: "image/png".into(),
            bytes: vec![1, 2, 3, 4, 5],
        }
    }

    fn settings(kind: DestKind) -> UploadSettings {
        UploadSettings {
            enabled: true,
            destinations: vec![Destination {
                id: "d1".into(),
                name: "Test".into(),
                enabled: true,
                kind,
            }],
            active: Some("d1".into()),
        }
    }

    #[test]
    fn nothing_is_sent_while_uploads_are_off() {
        let mock = Mock::new(200, "https://x/y");
        let store = MemoryStore::default();
        // defaults: master switch off
        let s = UploadSettings::default();
        assert!(matches!(
            upload(&s, None, &file(), &store, &mock),
            Err(UploadError::Disabled)
        ));
        // master on, destination off
        let mut s = settings(DestKind::Http(HttpDest {
            url: "https://up.example/api".into(),
            ..Default::default()
        }));
        s.destinations[0].enabled = false;
        assert!(matches!(
            upload(&s, None, &file(), &store, &mock),
            Err(UploadError::DestinationOff(_))
        ));
        // master off, destination on
        let mut s = settings(DestKind::Discord);
        s.enabled = false;
        assert!(matches!(
            upload(&s, None, &file(), &store, &mock),
            Err(UploadError::Disabled)
        ));
        assert!(mock.requests().is_empty(), "the transport was never called");
    }

    #[test]
    fn a_new_destination_defaults_to_off() {
        let d: Destination = serde_json::from_str(r#"{"id":"a","name":"A","kind":{"type":"discord"}}"#).unwrap();
        assert!(!d.enabled);
        assert!(!UploadSettings::default().enabled);
    }

    #[test]
    fn custom_http_sends_multipart_with_secret_header_and_reads_the_link() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = MemoryStore::default();
        store.set("d1.token", "tok-SECRET-1").unwrap();
        let dest = HttpDest {
            url: "https://up.example/api/upload".into(),
            headers: vec![("Authorization".into(), "Bearer {secret:token}".into())],
            fields: vec![("album".into(), "work".into())],
            json_path: "data.items[0].link".into(),
            ..Default::default()
        };
        let mock = Mock::new(200, r#"{"data":{"items":[{"link":"https://cdn.example/a.png"}]}}"#);
        let out = upload(&settings(DestKind::Http(dest)), None, &file(), &store, &mock).unwrap();
        assert_eq!(out.link.as_deref(), Some("https://cdn.example/a.png"));
        let r = &mock.requests()[0];
        assert_eq!(
            (r.method.as_str(), r.url.as_str()),
            ("POST", "https://up.example/api/upload")
        );
        assert!(
            r.headers
                .contains(&("Authorization".into(), "Bearer tok-SECRET-1".into()))
        );
        let ct = &r.headers.iter().find(|(k, _)| k == "Content-Type").unwrap().1;
        assert!(ct.starts_with("multipart/form-data; boundary="));
        let body = String::from_utf8_lossy(&r.body).into_owned();
        assert!(body.contains("name=\"album\"\r\n\r\nwork") && body.contains("name=\"file\"; filename=\"shot.png\""));
        crate::diag::clear_secrets();
    }

    #[test]
    fn raw_body_mode_and_plain_text_link() {
        let store = MemoryStore::default();
        let dest = HttpDest {
            url: "https://up.example/raw".into(),
            method: "put".into(),
            file_field: String::new(),
            ..Default::default()
        };
        let mock = Mock::new(201, "https://up.example/f/abc\n");
        let out = upload(&settings(DestKind::Http(dest)), None, &file(), &store, &mock).unwrap();
        assert_eq!(out.link.as_deref(), Some("https://up.example/f/abc"));
        let r = &mock.requests()[0];
        assert_eq!((r.method.as_str(), r.body.as_slice()), ("PUT", &[1u8, 2, 3, 4, 5][..]));
    }

    #[test]
    fn missing_secret_is_an_error_not_a_literal_placeholder() {
        let store = MemoryStore::default();
        let dest = HttpDest {
            url: "https://up.example".into(),
            headers: vec![("X-Key".into(), "{secret:token}".into())],
            ..Default::default()
        };
        let mock = Mock::new(200, "");
        assert!(matches!(
            upload(&settings(DestKind::Http(dest)), None, &file(), &store, &mock),
            Err(UploadError::Secret(_))
        ));
        assert!(mock.requests().is_empty());
    }

    #[test]
    fn server_errors_surface_without_leaking_secrets() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = MemoryStore::default();
        store.set("d1.token", "LEAKYTOKEN12345").unwrap();
        let dest = HttpDest {
            url: "https://up.example".into(),
            headers: vec![("Authorization".into(), "Bearer {secret:token}".into())],
            ..Default::default()
        };
        let mock = Mock::new(401, "bad token LEAKYTOKEN12345");
        let err = upload(&settings(DestKind::Http(dest)), None, &file(), &store, &mock).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("401") && !text.contains("LEAKYTOKEN12345"), "{text}");
        crate::diag::clear_secrets();
    }

    #[test]
    fn discord_posts_to_the_webhook_and_returns_the_attachment_url() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = MemoryStore::default();
        store
            .set("d1.webhook", "https://discord.com/api/webhooks/1/abcDEF")
            .unwrap();
        let mock = Mock::new(
            200,
            r#"{"attachments":[{"url":"https://cdn.discordapp.com/x/shot.png"}]}"#,
        );
        let out = upload(&settings(DestKind::Discord), None, &file(), &store, &mock).unwrap();
        assert_eq!(out.link.as_deref(), Some("https://cdn.discordapp.com/x/shot.png"));
        assert_eq!(
            mock.requests()[0].url,
            "https://discord.com/api/webhooks/1/abcDEF?wait=true"
        );
        crate::diag::clear_secrets();
    }

    #[test]
    fn telegram_uses_the_bot_api_and_builds_a_message_link_for_public_chats() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = MemoryStore::default();
        store.set("d1.token", "123456:ABC-def").unwrap();
        let mock = Mock::new(
            200,
            r#"{"ok":true,"result":{"message_id":77,"chat":{"username":"mychannel"}}}"#,
        );
        let out = upload(
            &settings(DestKind::Telegram {
                chat_id: "@mychannel".into(),
            }),
            None,
            &file(),
            &store,
            &mock,
        )
        .unwrap();
        assert_eq!(out.link.as_deref(), Some("https://t.me/mychannel/77"));
        assert_eq!(
            mock.requests()[0].url,
            "https://api.telegram.org/bot123456:ABC-def/sendDocument"
        );
        assert!(String::from_utf8_lossy(&mock.requests()[0].body).contains("name=\"chat_id\"\r\n\r\n@mychannel"));
        // a private chat has no public link, but the upload still succeeds
        let mock = Mock::new(200, r#"{"ok":true,"result":{"message_id":5,"chat":{"id":42}}}"#);
        let out = upload(
            &settings(DestKind::Telegram { chat_id: "42".into() }),
            None,
            &file(),
            &store,
            &mock,
        )
        .unwrap();
        assert_eq!(out.link, None);
        crate::diag::clear_secrets();
    }

    #[test]
    fn s3_put_is_signed_and_the_link_follows_the_public_base() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = MemoryStore::default();
        store.set("d1.access_key", "AKIAEXAMPLEKEY12345").unwrap();
        store.set("d1.secret_key", "secretsecretsecretsecret1234").unwrap();
        let d = S3Dest {
            endpoint: "https://s3.example.com/".into(),
            bucket: "shots".into(),
            region: "eu-west-1".into(),
            prefix: "screens/".into(),
            public_base: "https://cdn.example.com".into(),
            path_style: true,
        };
        let mock = Mock::new(200, "");
        let out = upload(&settings(DestKind::S3(d.clone())), None, &file(), &store, &mock).unwrap();
        assert_eq!(out.link.as_deref(), Some("https://cdn.example.com/screens/shot.png"));
        let r = &mock.requests()[0];
        assert_eq!(
            (r.method.as_str(), r.url.as_str()),
            ("PUT", "https://s3.example.com/shots/screens/shot.png")
        );
        let auth = &r.headers.iter().find(|(k, _)| k == "Authorization").unwrap().1;
        assert!(
            auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAEXAMPLEKEY12345/"),
            "{auth}"
        );
        assert!(
            auth.contains("/eu-west-1/s3/aws4_request")
                && auth.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date")
        );
        assert!(!auth.contains("secretsecret"), "the secret key is never sent");
        // virtual-hosted style
        let mut d2 = d;
        d2.path_style = false;
        d2.public_base.clear();
        let mock = Mock::new(200, "");
        let out = upload(&settings(DestKind::S3(d2)), None, &file(), &store, &mock).unwrap();
        assert_eq!(mock.requests()[0].url, "https://shots.s3.example.com/screens/shot.png");
        assert_eq!(
            out.link.as_deref(),
            Some("https://shots.s3.example.com/screens/shot.png")
        );
        crate::diag::clear_secrets();
    }

    #[test]
    fn json_paths() {
        let j = r#"{"a":{"b":[{"c":"x"},{"c":"y"}],"n":5},"z":null}"#;
        assert_eq!(json_path(j, "a.b[1].c").as_deref(), Some("y"));
        assert_eq!(json_path(j, "a.n").as_deref(), Some("5"));
        assert_eq!(json_path(j, "z"), None);
        assert_eq!(json_path(j, "a.missing"), None);
        assert_eq!(json_path("not json", "a"), None);
    }

    #[test]
    fn multipart_layout() {
        let body = multipart("BND", &[("k".into(), "v".into())], "file", &file());
        let s = String::from_utf8_lossy(&body);
        assert!(s.starts_with("--BND\r\nContent-Disposition: form-data; name=\"k\"\r\n\r\nv\r\n--BND\r\n"));
        assert!(s.ends_with("\r\n--BND--\r\n"));
        assert!(s.contains("Content-Type: image/png"));
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for("a.JPG"), "image/jpeg");
        assert_eq!(mime_for("a.png"), "image/png");
        assert_eq!(mime_for("a"), "application/octet-stream");
    }
}
