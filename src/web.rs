//! Serves the Hyperlight Configurator (the files in web/, built into the exe) on
//! localhost, so "Open Configurator" works with no hosting and no install.
//!
//! WebHID only runs on secure origins, and http://localhost counts as one. The
//! server binds to the loopback address only, answers GET/HEAD for the embedded
//! files, and rejects requests whose Host header isn't localhost (which blocks
//! DNS-rebinding tricks from other websites). It starts on first use and runs on
//! one background thread until the app exits.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::wide;

/// A fixed port keeps the page's origin stable, so the browser remembers which
/// device you allowed and the page's own settings. Neighbours are tried if busy.
const PORTS: [u16; 5] = [47820, 47821, 47822, 47823, 47824];

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

fn handle(mut s: TcpStream, port: u16) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = [0u8; 8192];
    let mut len = 0;
    // Read until the end of the request headers.
    while len < buf.len() {
        let n = s.read(&mut buf[len..])?;
        if n == 0 {
            break;
        }
        len += n;
        if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let req = String::from_utf8_lossy(&buf[..len]);
    let mut lines = req.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or("/"));
    let host = lines
        .take_while(|l| !l.is_empty())
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("host")).map(|(_, v)| v.trim()))
        .unwrap_or("");

    let with_body = method != "HEAD";
    let allowed_hosts = [format!("localhost:{port}"), format!("127.0.0.1:{port}")];
    if !allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return respond(&mut s, "403 Forbidden", "text/plain", b"Forbidden", with_body);
    }
    if method != "GET" && method != "HEAD" {
        return respond(&mut s, "405 Method Not Allowed", "text/plain", b"Method not allowed", true);
    }
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let path = if path == "/" { "/index.html" } else { path };
    match FILES.iter().find(|f| f.path == path) {
        Some(f) => respond(&mut s, "200 OK", f.mime, f.body, with_body),
        None => respond(&mut s, "404 Not Found", "text/plain", b"Not found", with_body),
    }
}

fn respond(s: &mut TcpStream, status: &str, mime: &str, body: &[u8], with_body: bool) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n\
         Cache-Control: no-cache\r\nX-Content-Type-Options: nosniff\r\n\
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

    fn get(port: u16, path: &str, host: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
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
    }
}
