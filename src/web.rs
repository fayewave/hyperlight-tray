//! Serves the Hyperlight Configurator (the files in web/, built into the exe) on
//! localhost, so "Open Configurator" works with no hosting and no install. It also
//! exposes the tray's own settings to the page's Tray tab.
//!
//! WebHID only runs on secure origins, and http://localhost counts as one. Safety:
//! - binds to the loopback address only
//! - refuses requests whose Host header isn't localhost (blocks DNS rebinding)
//! - changes (POST) must come from the page itself: the Origin header has to be this
//!   server, and the body must be JSON, which other sites can't send without a CORS
//!   preflight that this server never approves
//!
//! It starts on first use and runs on one background thread until the app exits.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, WM_APP};

use crate::wide;

/// A fixed port keeps the page's origin stable, so the browser remembers which
/// device you allowed and the page's own settings. Neighbours are tried if busy.
const PORTS: [u16; 5] = [47820, 47821, 47822, 47823, 47824];

/// Message the server thread sends to the tray window to read or change settings.
/// `lParam` points at an [`ApiRequest`], which the window fills in.
pub const WM_TRAY_API: u32 = WM_APP + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TrayInfo {
    pub threshold: u8,
    pub notify_full: bool,
    pub autostart: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ApiCall {
    Get,
    Set { threshold: Option<u8>, notify_full: Option<bool>, autostart: Option<bool> },
    TestNotification,
}

pub struct ApiRequest {
    pub call: ApiCall,
    /// Filled in by the tray window: the settings after the call.
    pub result: Option<TrayInfo>,
}

static WINDOW: AtomicIsize = AtomicIsize::new(0);

/// Tells the server which window answers settings requests.
pub fn set_window(hwnd: *mut core::ffi::c_void) {
    WINDOW.store(hwnd as isize, Ordering::Relaxed);
}

fn ask_tray(call: ApiCall) -> Option<TrayInfo> {
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        return None;
    }
    let mut req = ApiRequest { call, result: None };
    let mut out = 0usize;
    // Synchronous: the window handles it on the UI thread while we wait.
    unsafe {
        SendMessageTimeoutW(
            hwnd as _,
            WM_TRAY_API,
            0,
            &mut req as *mut ApiRequest as isize,
            SMTO_ABORTIFHUNG,
            3000,
            &mut out,
        );
    }
    req.result
}

struct File {
    path: &'static str,
    mime: &'static str,
    body: &'static [u8],
}

macro_rules! file {
    ($path:literal, $mime:literal) => {
        File { path: $path, mime: $mime, body: include_bytes!(concat!("../web", $path)) }
    };
}

static FILES: &[File] = &[
    file!("/index.html", "text/html; charset=utf-8"),
    file!("/favicon.svg", "image/svg+xml"),
    file!("/css/app.css", "text/css; charset=utf-8"),
    file!("/js/app.js", "text/javascript; charset=utf-8"),
    file!("/js/protocol.js", "text/javascript; charset=utf-8"),
    file!("/js/device.js", "text/javascript; charset=utf-8"),
    file!("/js/demo.js", "text/javascript; charset=utf-8"),
];

static PORT: OnceLock<Option<u16>> = OnceLock::new();

/// Starts the server if needed and opens the page in the default browser.
/// Returns false if no port could be opened.
pub fn open() -> bool {
    let Some(port) = *PORT.get_or_init(start) else { return false };
    let url = wide(&format!("http://localhost:{port}/"));
    let verb = wide("open");
    unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
    true
}

fn start() -> Option<u16> {
    let (listener, port) = PORTS
        .iter()
        .find_map(|&p| TcpListener::bind((Ipv4Addr::LOCALHOST, p)).ok().map(|l| (l, p)))?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = handle(stream, port);
        }
    });
    Some(port)
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

fn read_request(s: &mut TcpStream) -> std::io::Result<Request> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break buf.len();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 16 * 1024 {
            break buf.len();
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end.min(buf.len())]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let target = first.next().unwrap_or("/");
    let path = target.split(['?', '#']).next().unwrap_or("/").to_string();
    let headers: Vec<(String, String)> = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let want: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0)
        .min(4096);
    let mut body = buf[head_end.min(buf.len())..].to_vec();
    while body.len() < want {
        let n = s.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(want);
    Ok(Request { method, path, headers, body })
}

fn handle(mut s: TcpStream, port: u16) -> std::io::Result<()> {
    let req = read_request(&mut s)?;
    let with_body = req.method != "HEAD";
    let local = [format!("localhost:{port}"), format!("127.0.0.1:{port}")];
    if !local.iter().any(|h| h.eq_ignore_ascii_case(req.header("host"))) {
        return respond(&mut s, "403 Forbidden", "text/plain", b"Forbidden", with_body);
    }

    if req.path.starts_with("/api/") {
        return api(&mut s, &req, &local);
    }

    if req.method != "GET" && req.method != "HEAD" {
        return respond(&mut s, "405 Method Not Allowed", "text/plain", b"Method not allowed", true);
    }
    let path = if req.path == "/" { "/index.html" } else { req.path.as_str() };
    match FILES.iter().find(|f| f.path == path) {
        Some(f) => respond(&mut s, "200 OK", f.mime, f.body, with_body),
        None => respond(&mut s, "404 Not Found", "text/plain", b"Not found", with_body),
    }
}

fn api(s: &mut TcpStream, req: &Request, local: &[String; 2]) -> std::io::Result<()> {
    const JSON: &str = "application/json";
    let call = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/tray") => ApiCall::Get,
        ("POST", "/api/tray") | ("POST", "/api/tray/test") => {
            // Only the page itself may change things.
            let origin_ok = local.iter().any(|h| req.header("origin").eq_ignore_ascii_case(&format!("http://{h}")));
            if !origin_ok || !req.header("content-type").starts_with(JSON) {
                return respond(s, "403 Forbidden", "text/plain", b"Forbidden", true);
            }
            if req.path == "/api/tray/test" {
                ApiCall::TestNotification
            } else {
                let body = String::from_utf8_lossy(&req.body);
                ApiCall::Set {
                    threshold: json_value(&body, "threshold").and_then(|v| v.parse::<u8>().ok()).filter(|t| *t <= 50),
                    notify_full: json_value(&body, "notifyFull").and_then(parse_bool),
                    autostart: json_value(&body, "autostart").and_then(parse_bool),
                }
            }
        }
        _ => return respond(s, "404 Not Found", "text/plain", b"Not found", true),
    };
    match ask_tray(call) {
        Some(info) => respond(s, "200 OK", JSON, info_json(&info).as_bytes(), true),
        None => respond(s, "503 Service Unavailable", "text/plain", b"Tray not ready", true),
    }
}

fn info_json(i: &TrayInfo) -> String {
    format!(
        "{{\"version\":\"{}\",\"threshold\":{},\"notifyFull\":{},\"autostart\":{}}}",
        env!("CARGO_PKG_VERSION"),
        i.threshold,
        i.notify_full,
        i.autostart
    )
}

/// The raw text of a top-level number or boolean in a flat JSON object.
fn json_value<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let rest = &body[body.find(&pat)? + pat.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn parse_bool(v: &str) -> Option<bool> {
    match v {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn respond(s: &mut TcpStream, status: &str, mime: &str, body: &[u8], with_body: bool) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    if with_body {
        s.write_all(body)?;
    }
    s.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(port: u16, raw: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    fn get(port: u16, path: &str, host: &str) -> String {
        send(port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"))
    }

    #[test]
    fn serves_embedded_page_on_localhost_only() {
        let port = start().expect("a free port");
        let host = format!("localhost:{port}");
        let page = get(port, "/", &host);
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("<title>Hyperlight Configurator</title>"));
        assert!(get(port, "/js/app.js?v=1", &host).contains("Content-Type: text/javascript"));
        assert!(get(port, "/../Cargo.toml", &host).starts_with("HTTP/1.1 404"));
        // A page on another site pointing its own domain at 127.0.0.1 is refused.
        assert!(get(port, "/", "evil.example:47820").starts_with("HTTP/1.1 403"));

        // Settings changes must come from the page itself, as JSON.
        let body = r#"{"autostart":true}"#;
        let post = |origin: &str, ctype: &str| {
            send(port, &format!(
                "POST /api/tray HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ))
        };
        assert!(post("https://evil.example", "application/json").starts_with("HTTP/1.1 403"));
        assert!(post(&format!("http://{host}"), "text/plain").starts_with("HTTP/1.1 403"));
        // No tray window in tests, so an allowed request reaches the tray and gets 503.
        assert!(post(&format!("http://{host}"), "application/json").starts_with("HTTP/1.1 503"));
    }

    #[test]
    fn json_values() {
        let b = r#"{ "threshold": 15, "notifyFull" :false,"autostart":true }"#;
        assert_eq!(json_value(b, "threshold"), Some("15"));
        assert_eq!(json_value(b, "notifyFull").and_then(parse_bool), Some(false));
        assert_eq!(json_value(b, "autostart").and_then(parse_bool), Some(true));
        assert_eq!(json_value(b, "missing"), None);
    }
}
