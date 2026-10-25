//! In-memory, window-scoped preview capabilities. No paths or credentials enter a URL.
use std::{
    borrow::Cow,
    collections::HashMap,
    io,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use wry::http::{self, Request, Response};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ITEM: usize = 16 * 1024 * 1024;
const TTL: Duration = Duration::from_secs(600);
#[derive(Clone)]
pub struct Ticket {
    page: String,
    nonce: String,
}
struct Asset {
    token: String,
    bytes: Vec<u8>,
    mime: &'static str,
    at: Instant,
}
struct Page {
    owner: String,
    connection: String,
    nonce: String,
    asset: Option<Asset>,
}
#[derive(Default)]
struct Registry {
    pages: HashMap<String, Page>,
}
fn registry() -> &'static Mutex<Registry> {
    static STATE: OnceLock<Mutex<Registry>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}
fn random() -> io::Result<String> {
    let mut bytes = [0; 24];
    getrandom::getrandom(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
impl Registry {
    fn bind(&mut self, page: &str, owner: &str) -> bool {
        if page.is_empty() || page.len() > 128 {
            return false;
        }
        if let Some(p) = self.pages.get(page) {
            return p.owner == owner;
        }
        self.pages.retain(|_, p| p.owner != owner);
        if self.pages.len() >= 128 {
            return false;
        }
        self.pages.insert(
            page.into(),
            Page {
                owner: owner.into(),
                connection: String::new(),
                nonce: String::new(),
                asset: None,
            },
        );
        true
    }
    fn begin(&mut self, page: &str, connection: &str) -> io::Result<Ticket> {
        let p = self
            .pages
            .get_mut(page)
            .ok_or_else(|| io::Error::other("파일 창이 닫혔습니다"))?;
        p.nonce = random()?;
        p.connection = connection.into();
        p.asset = None;
        Ok(Ticket {
            page: page.into(),
            nonce: p.nonce.clone(),
        })
    }
    fn publish(&mut self, ticket: &Ticket, bytes: Vec<u8>, mime: &str) -> io::Result<String> {
        let mime = media_mime(mime)
            .ok_or_else(|| io::Error::other("지원하지 않는 미리보기 형식입니다"))?;
        if bytes.len() > MAX_ITEM {
            return Err(io::Error::other("미리보기 크기 제한을 넘었습니다"));
        }
        if !self
            .pages
            .get(&ticket.page)
            .is_some_and(|p| p.nonce == ticket.nonce)
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "이전 미리보기입니다",
            ));
        }
        self.prune();
        while self
            .pages
            .values()
            .filter_map(|p| p.asset.as_ref())
            .map(|a| a.bytes.len())
            .sum::<usize>()
            + bytes.len()
            > MAX_BYTES
        {
            let Some(oldest) = self
                .pages
                .iter()
                .filter_map(|(id, p)| p.asset.as_ref().map(|a| (id.clone(), a.at)))
                .min_by_key(|(_, at)| *at)
                .map(|(id, _)| id)
            else {
                break;
            };
            self.pages.get_mut(&oldest).unwrap().asset = None;
        }
        let token = random()?;
        self.pages.get_mut(&ticket.page).unwrap().asset = Some(Asset {
            token: token.clone(),
            bytes,
            mime,
            at: Instant::now(),
        });
        Ok(format!("/preview/{token}"))
    }
    fn prune(&mut self) {
        for p in self.pages.values_mut() {
            if p.asset.as_ref().is_some_and(|a| a.at.elapsed() >= TTL) {
                p.asset = None;
            }
        }
    }
    fn response(&mut self, owner: &str, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        self.prune();
        let Some(token) = req.uri().path().strip_prefix("/preview/") else {
            return empty(404, None);
        };
        let Some(a) = self
            .pages
            .values()
            .filter(|p| p.owner == owner)
            .filter_map(|p| p.asset.as_ref())
            .find(|a| a.token == token)
        else {
            return empty(404, None);
        };
        if req.method() != "GET" && req.method() != "HEAD" {
            return empty(405, None);
        }
        let len = a.bytes.len();
        let range = if let Some(range) = req.headers().get("Range") {
            match range.to_str().ok().and_then(|r| byte_range(r, len)) {
                Some(range) => Some(range),
                None => return empty(416, Some(format!("bytes */{len}"))),
            }
        } else {
            None
        };
        let (start, end) = range.unwrap_or((0, len));
        let mut b = headers(if range.is_some() { 206 } else { 200 })
            .header("Content-Type", a.mime)
            .header("Accept-Ranges", "bytes")
            .header("Content-Length", end - start);
        if range.is_some() {
            b = b.header("Content-Range", format!("bytes {start}-{}/{len}", end - 1));
        }
        b.body(if req.method() == "HEAD" {
            Cow::Borrowed(&b""[..])
        } else {
            Cow::Owned(a.bytes[start..end].to_vec())
        })
        .unwrap()
    }
}
fn headers(status: u16) -> http::response::Builder {
    Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "default-src 'none'; sandbox")
}
fn empty(status: u16, range: Option<String>) -> Response<Cow<'static, [u8]>> {
    let mut b = headers(status).header("Content-Length", "0");
    if let Some(range) = range {
        b = b.header("Content-Range", range);
    }
    b.body(Cow::Borrowed(&b""[..])).unwrap()
}
fn media_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/bmp" => "image/bmp",
        "image/avif" => "image/avif",
        "video/mp4" => "video/mp4",
        "video/webm" => "video/webm",
        "audio/mpeg" => "audio/mpeg",
        "audio/wav" => "audio/wav",
        "audio/ogg" => "audio/ogg",
        _ => return None,
    })
}
/// A single HTTP byte range, converted to an exclusive end. Multipart requests are refused.
fn byte_range(value: &str, len: usize) -> Option<(usize, usize)> {
    if len == 0 {
        return None;
    }
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if !start.bytes().all(|b| b.is_ascii_digit()) || !end.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if start.is_empty() {
        let count = end.parse::<usize>().ok()?;
        return (count > 0).then_some((len.saturating_sub(count), len));
    }
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        len
    } else {
        end.parse::<usize>().ok()?.checked_add(1)?.min(len)
    };
    (start < end && start < len).then_some((start, end))
}
pub fn bind_page(page: &str, owner: &str) -> bool {
    registry().lock().unwrap().bind(page, owner)
}
/// Bind only from the native callback, whose owner cannot be chosen by JavaScript.
pub fn admit_ipc(v: &serde_json::Value, owner: &str) -> bool {
    match v["cmd"].as_str() {
        Some("files") => bind_page(v["window"].as_str().unwrap_or(""), owner),
        Some("files_preview_release") => {
            if let Some(p) = registry()
                .lock()
                .unwrap()
                .pages
                .get_mut(v["page"].as_str().unwrap_or(""))
            {
                if p.owner == owner {
                    p.asset = None;
                    p.nonce.clear();
                }
            }
            false
        }
        _ => true,
    }
}
pub fn owner(page: &str) -> Option<String> {
    registry()
        .lock()
        .unwrap()
        .pages
        .get(page)
        .map(|p| p.owner.clone())
}
pub fn begin(page: &str, connection: &str) -> io::Result<Ticket> {
    registry().lock().unwrap().begin(page, connection)
}
pub fn publish(ticket: &Ticket, bytes: Vec<u8>, mime: &str) -> io::Result<String> {
    registry().lock().unwrap().publish(ticket, bytes, mime)
}
pub fn revoke_page(page: &str) {
    registry().lock().unwrap().pages.remove(page);
}
pub fn revoke_owner(owner: &str) {
    registry()
        .lock()
        .unwrap()
        .pages
        .retain(|_, p| p.owner != owner);
}
pub fn revoke_connection(key: &str) {
    for p in registry().lock().unwrap().pages.values_mut() {
        if p.connection == key
            || (!key.contains('\x1f') && p.connection.starts_with(&format!("{key}\x1f")))
        {
            p.asset = None;
            p.nonce.clear();
        }
    }
}
pub fn response(owner: &str, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    registry().lock().unwrap().response(owner, req)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn req(path: &str, range: Option<&str>, method: &str) -> Request<Vec<u8>> {
        let mut b = Request::builder().uri(path).method(method);
        if let Some(r) = range {
            b = b.header("Range", r);
        }
        b.body(Vec::new()).unwrap()
    }
    #[test]
    fn capability_is_window_scoped_and_revoked_on_reload_and_new_selection() {
        let mut r = Registry::default();
        assert!(r.bind("a", "window-a"));
        assert!(!r.bind("a", "window-b"));
        let old = r.begin("a", "device").unwrap();
        let ticket = r.begin("a", "device").unwrap();
        assert!(r.publish(&old, vec![0], "image/png").is_err());
        let url = r.publish(&ticket, vec![1, 2, 3], "image/png").unwrap();
        assert_eq!(
            r.response("window-b", &req(&url, None, "GET")).status(),
            404
        );
        assert_eq!(
            r.response("window-a", &req(&url, None, "GET"))
                .body()
                .as_ref(),
            &[1, 2, 3]
        );
        assert!(r.bind("replacement", "window-a"));
        assert_eq!(
            r.response("window-a", &req(&url, None, "GET")).status(),
            404
        );
        assert!(r.publish(&ticket, vec![1], "image/png").is_err());
    }
    #[test]
    fn ranges_and_head_have_exact_bytes_and_lengths() {
        let mut r = Registry::default();
        r.bind("p", "w");
        let t = r.begin("p", "c").unwrap();
        let url = r.publish(&t, (0..10).collect(), "video/mp4").unwrap();
        for (range, bytes, content) in [
            ("bytes=2-4", vec![2, 3, 4], "bytes 2-4/10"),
            ("bytes=-2", vec![8, 9], "bytes 8-9/10"),
            ("bytes=8-99", vec![8, 9], "bytes 8-9/10"),
            ("bytes=7-", vec![7, 8, 9], "bytes 7-9/10"),
        ] {
            let resp = r.response("w", &req(&url, Some(range), "GET"));
            assert_eq!(resp.status(), 206);
            assert_eq!(resp.body().as_ref(), bytes);
            assert_eq!(resp.headers()["Content-Range"], content);
            let head = r.response("w", &req(&url, Some(range), "HEAD"));
            assert!(head.body().is_empty());
            assert_eq!(head.headers()["Content-Length"], bytes.len().to_string());
        }
        for bad in [
            "bytes=10-",
            "bytes=5-2",
            "bytes=-0",
            "bytes=0-1,4-5",
            "bytes=0-18446744073709551615",
            "garbage",
        ] {
            let resp = r.response("w", &req(&url, Some(bad), "GET"));
            assert_eq!(resp.status(), 416);
            assert_eq!(resp.headers()["Content-Range"], "bytes */10");
        }
        assert_eq!(r.response("w", &req(&url, None, "POST")).status(), 405);
    }
    #[test]
    fn only_inert_media_and_bounded_live_capabilities() {
        let mut r = Registry::default();
        r.bind("p", "w");
        let t = r.begin("p", "c").unwrap();
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/pdf",
            "image/png\r\nX: bad",
        ] {
            assert!(r.publish(&t, vec![1], mime).is_err());
        }
        assert!(r.publish(&t, vec![0; MAX_ITEM + 1], "image/png").is_err());
        let url = r.publish(&t, vec![], "image/png").unwrap();
        assert_eq!(
            r.response("w", &req(&url, Some("bytes=0-"), "GET"))
                .status(),
            416
        );
        r.pages.get_mut("p").unwrap().asset.as_mut().unwrap().at = Instant::now() - TTL;
        assert_eq!(r.response("w", &req(&url, None, "GET")).status(), 404);
    }
    #[test]
    fn retained_payloads_are_bounded_across_windows() {
        let mut r = Registry::default();
        let mut urls = Vec::new();
        for i in 0..5 {
            let p = format!("page-{i}");
            let w = format!("window-{i}");
            r.bind(&p, &w);
            let t = r.begin(&p, "c").unwrap();
            urls.push(r.publish(&t, vec![0; MAX_ITEM], "image/png").unwrap());
        }
        assert_eq!(
            r.pages
                .values()
                .filter_map(|p| p.asset.as_ref())
                .map(|a| a.bytes.len())
                .sum::<usize>(),
            MAX_BYTES
        );
        assert_eq!(
            r.response("window-0", &req(&urls[0], None, "HEAD"))
                .status(),
            404
        );
        let last = r.response("window-4", &req(&urls[4], None, "HEAD"));
        assert_eq!(last.status(), 200);
        assert_eq!(last.headers()["Content-Length"], MAX_ITEM.to_string());
        assert_eq!(last.headers()["X-Content-Type-Options"], "nosniff");
    }
    #[test]
    fn native_release_cannot_revoke_another_windows_capability() {
        let page = random().unwrap();
        let owner = random().unwrap();
        assert!(bind_page(&page, &owner));
        let t = begin(&page, "c").unwrap();
        let url = publish(&t, vec![42], "image/png").unwrap();
        let release = serde_json::json!({"cmd":"files_preview_release","page":page});
        assert!(!admit_ipc(&release, "foreign"));
        assert_eq!(response(&owner, &req(&url, None, "GET")).status(), 200);
        assert!(!admit_ipc(&release, &owner));
        assert_eq!(response(&owner, &req(&url, None, "GET")).status(), 404);
        assert!(publish(&t, vec![42], "image/png").is_err());
        revoke_owner(&owner);
    }
}
