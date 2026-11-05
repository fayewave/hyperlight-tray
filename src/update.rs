//! "Update available" check against the project's GitHub releases.
//!
//! One anonymous HTTPS request to GitHub's public API (via WinHTTP, so the OS
//! handles TLS and proxies), asking for the latest published release's tag. If
//! it's newer than this build, the tray and configurator point people at the
//! release page. Nothing about the user or their mouse is sent.

use std::ptr;

use windows_sys::Win32::Networking::WinHttp::*;

/// GitHub "owner/name" this build checks for releases.
pub const REPO: &str = "fayewave/hyperlight-tray";

/// The page the update link opens: always the newest release.
pub fn release_page() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// "v1.2.3", "1.2", "v0.10.0-beta" -> (major, minor, patch). Pre-release text is ignored.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// Whether `tag` is a newer version than `current`.
pub fn is_newer(current: &str, tag: &str) -> bool {
    match (parse_version(current), parse_version(tag)) {
        (Some(c), Some(t)) => t > c,
        _ => false,
    }
}

/// A tag made only of version-like characters, safe to show and to put in JSON.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() < 40 && tag.chars().all(|c| c.is_ascii_alphanumeric() || ".-+_".contains(c))
}

/// The `tag_name` of a GitHub release JSON object.
pub fn tag_from_json(json: &str) -> Option<String> {
    let rest = &json[json.find("\"tag_name\"")? + 10..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let tag = &rest[..rest.find('"')?];
    valid_tag(tag).then(|| tag.to_string())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Fetches the latest release's tag for `repo` ("owner/name"). Blocking, with
/// short timeouts; None on any failure (offline, private repo, no releases).
pub fn latest_tag(repo: &str) -> Option<String> {
    unsafe {
        let agent = wide(&format!("hyperlight-tray/{}", env!("CARGO_PKG_VERSION")));
        let session = WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, ptr::null(), ptr::null(), 0);
        if session.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session, 5000, 5000, 5000, 8000);
        let result = (|| {
            let host = wide("api.github.com");
            let conn = WinHttpConnect(session, host.as_ptr(), 443, 0);
            if conn.is_null() {
                return None;
            }
            let path = wide(&format!("/repos/{repo}/releases/latest"));
            let verb = wide("GET");
            let req = WinHttpOpenRequest(conn, verb.as_ptr(), path.as_ptr(), ptr::null(), ptr::null(), ptr::null(), WINHTTP_FLAG_SECURE);
            let body = if req.is_null() {
                None
            } else {
                let headers = wide("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28");
                let mut out = None;
                if WinHttpSendRequest(req, headers.as_ptr(), u32::MAX, ptr::null(), 0, 0, 0) != 0
                    && WinHttpReceiveResponse(req, ptr::null_mut()) != 0
                {
                    let mut status = 0u32;
                    let mut len = 4u32;
                    WinHttpQueryHeaders(
                        req,
                        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                        ptr::null(),
                        &mut status as *mut u32 as _,
                        &mut len,
                        ptr::null_mut(),
                    );
                    if status == 200 {
                        let mut data = Vec::new();
                        loop {
                            let mut avail = 0u32;
                            if WinHttpQueryDataAvailable(req, &mut avail) == 0 || avail == 0 || data.len() > 512 * 1024 {
                                break;
                            }
                            let start = data.len();
                            data.resize(start + avail as usize, 0);
                            let mut read = 0u32;
                            if WinHttpReadData(req, data[start..].as_mut_ptr() as _, avail, &mut read) == 0 {
                                break;
                            }
                            data.truncate(start + read as usize);
                        }
                        out = Some(String::from_utf8_lossy(&data).into_owned());
                    }
                }
                WinHttpCloseHandle(req);
                out
            };
            WinHttpCloseHandle(conn);
            body
        })();
        WinHttpCloseHandle(session);
        tag_from_json(&result?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.3"), Some((1, 3, 0)));
        assert_eq!(parse_version("v0.10.1-beta.2"), Some((0, 10, 1)));
        assert_eq!(parse_version("nightly"), None);
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("0.9.0", "v0.10.0"));
        assert!(!is_newer("0.1.0", "v0.1.0"));
        assert!(!is_newer("0.2.0", "v0.1.9"));
        assert!(!is_newer("0.1.0", "garbage"));
    }

    #[test]
    fn reads_tag_name() {
        let json = r#"{"url":"https://api.github.com/x","html_url":"https://github.com/a/b/releases/tag/v0.2.0","id":1,"tag_name": "v0.2.0","name":"v0.2.0"}"#;
        assert_eq!(tag_from_json(json).as_deref(), Some("v0.2.0"));
        assert_eq!(tag_from_json(r#"{"message":"Not Found"}"#), None);
        assert_eq!(tag_from_json(r#"{"tag_name":"v1.0.0<script>"}"#), None);
    }

    /// Real request against a public repo: `cargo test --lib update -- --ignored`
    #[test]
    #[ignore]
    fn live_request() {
        let tag = latest_tag("BurntSushi/ripgrep");
        println!("BurntSushi/ripgrep latest: {tag:?}");
        assert!(tag.is_some());
        // This project's repo is private for now, so there's nothing to find yet.
        println!("{REPO} latest: {:?}", latest_tag(REPO));
    }
}
