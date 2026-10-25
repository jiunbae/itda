//! In-memory, window-scoped preview capabilities. No paths or credentials enter a URL.
use std::{
    borrow::Cow,
    collections::HashMap,
    io,
    sync::{Arc, Mutex, OnceLock},
};
use wry::http::{self, Request, Response};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ITEM: usize = 16 * 1024 * 1024;
#[derive(Clone)]
pub struct Ticket {
    page: String,
    nonce: String,
}
struct Asset {
    token: String,
    bytes: Arc<[u8]>,
    mime: &'static str,
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
    fn publish(
        &mut self,
        ticket: &Ticket,
        bytes: impl Into<Arc<[u8]>>,
        mime: &str,
    ) -> io::Result<String> {
        let bytes = bytes.into();
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
        // A currently displayed audio/video may request a later range after a long pause.
        // Never evict its URL behind the player's back; reject new work at the active budget.
        if self
            .pages
            .iter()
            .filter(|(page, _)| *page != &ticket.page)
            .filter_map(|(_, p)| p.asset.as_ref())
            .map(|a| a.bytes.len())
            .sum::<usize>()
            + bytes.len()
            > MAX_BYTES
        {
            return Err(io::Error::other(
                "다른 파일 창의 미리보기를 닫은 뒤 다시 선택해 주세요",
            ));
        }
        let token = random()?;
        self.pages.get_mut(&ticket.page).unwrap().asset = Some(Asset {
            token: token.clone(),
            bytes,
            mime,
        });
        Ok(format!("/preview/{token}"))
    }
    fn revoke_owner(&mut self, owner: &str) {
        self.pages.retain(|_, p| p.owner != owner);
    }
    fn resolve(&mut self, owner: &str, req: &Request<Vec<u8>>) -> Snapshot {
        let Some(token) = req.uri().path().strip_prefix("/preview/") else {
            return Snapshot::Empty(404, None);
        };
        let Some(page) = self
            .pages
            .values_mut()
            .find(|p| p.owner == owner && p.asset.as_ref().is_some_and(|a| a.token == token))
        else {
            return Snapshot::Empty(404, None);
        };
        if req.method() != "GET" && req.method() != "HEAD" {
            return Snapshot::Empty(405, None);
        }
        let a = page.asset.as_ref().unwrap();
        let len = a.bytes.len();
        // Range is defined for GET; ignore unsupported/malformed/multiple ranges.
        let range = if req.method() == "GET" && !req.headers().contains_key("If-Range") {
            let mut values = req.headers().get_all("Range").iter();
            match (values.next(), values.next()) {
                (Some(v), None) => v
                    .to_str()
                    .map(|s| byte_range(s, len))
                    .unwrap_or(ByteRange::Full),
                _ => ByteRange::Full,
            }
        } else {
            ByteRange::Full
        };
        if matches!(range, ByteRange::Unsatisfiable) {
            return Snapshot::Empty(416, Some(format!("bytes */{len}")));
        }
        let result = Snapshot::Data {
            bytes: a.bytes.clone(),
            mime: a.mime,
            range,
            head: req.method() == "HEAD",
        };
        // A decoded image retains its own pixels. Unlike an audio/video player, it doesn't
        // need a future range read. The Arc keeps this response alive after detaching it.
        if req.method() == "GET" && range == ByteRange::Full && a.mime.starts_with("image/") {
            page.asset = None;
        }
        result
    }
    #[cfg(test)]
    fn response(&mut self, owner: &str, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        self.resolve(owner, req).into_response()
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
    if status == 405 {
        b = b.header("Allow", "GET, HEAD");
    }
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ByteRange {
    Full,
    Partial(usize, usize),
    Unsatisfiable,
}
enum Snapshot {
    Empty(u16, Option<String>),
    Data {
        bytes: Arc<[u8]>,
        mime: &'static str,
        range: ByteRange,
        head: bool,
    },
}
impl Snapshot {
    fn into_response(self) -> Response<Cow<'static, [u8]>> {
        match self {
            Self::Empty(status, range) => empty(status, range),
            Self::Data {
                bytes,
                mime,
                range,
                head,
            } => {
                let len = bytes.len();
                let (status, start, end) = match range {
                    ByteRange::Partial(start, end) => (206, start, end),
                    _ => (200, 0, len),
                };
                let mut builder = headers(status)
                    .header("Content-Type", mime)
                    .header("Accept-Ranges", "bytes")
                    .header("Content-Length", end - start);
                if status == 206 {
                    builder =
                        builder.header("Content-Range", format!("bytes {start}-{}/{len}", end - 1));
                }
                builder
                    .body(if head {
                        Cow::Borrowed(&b""[..])
                    } else {
                        Cow::Owned(bytes[start..end].to_vec())
                    })
                    .unwrap()
            }
        }
    }
}
/// Single byte ranges with saturating numeral parsing (RFC 9110 sections 14.1.2/14.2).
/// Unsupported/malformed range sets are ignored; only valid unsatisfiable ranges yield 416.
fn byte_range(value: &str, len: usize) -> ByteRange {
    let Some((start, end)) = value
        .trim()
        .strip_prefix("bytes=")
        .and_then(|v| v.split_once('-'))
    else {
        return ByteRange::Full;
    };
    let number = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| {
            s.bytes().fold(0usize, |n, b| {
                n.saturating_mul(10).saturating_add((b - b'0') as usize)
            })
        })
    };
    if start.is_empty() {
        let Some(count) = number(end) else {
            return ByteRange::Full;
        };
        return if count == 0 || len == 0 {
            ByteRange::Unsatisfiable
        } else {
            ByteRange::Partial(len.saturating_sub(count), len)
        };
    }
    let Some(first) = number(start) else {
        return ByteRange::Full;
    };
    let last = if end.is_empty() {
        None
    } else {
        let Some(last) = number(end) else {
            return ByteRange::Full;
        };
        let (a, b) = (start.trim_start_matches('0'), end.trim_start_matches('0'));
        if (a.len(), a) > (b.len(), b) {
            return ByteRange::Full;
        }
        Some(last)
    };
    if first >= len {
        return ByteRange::Unsatisfiable;
    }
    ByteRange::Partial(first, last.map_or(len, |n| n.min(len - 1) + 1))
}
pub fn bind_page(page: &str, owner: &str) -> bool {
    registry().lock().unwrap().bind(page, owner)
}
/// Bind only from the native callback, whose owner cannot be chosen by JavaScript.
pub fn admit_ipc(v: &serde_json::Value, owner: &str) -> bool {
    match v["cmd"].as_str() {
        Some("files") => bind_page(v["window"].as_str().unwrap_or(""), owner),
        Some("files_release_page") => {
            self::owner(v["page"].as_str().unwrap_or("")).as_deref() == Some(owner)
        }
        Some("files_preview_release") => {
            if let Some(p) = registry()
                .lock()
                .unwrap()
                .pages
                .get_mut(v["page"].as_str().unwrap_or(""))
            {
                let token_matches = v["url"].as_str().is_none_or(|url| {
                    p.asset
                        .as_ref()
                        .is_some_and(|a| url.strip_prefix("/preview/") == Some(a.token.as_str()))
                });
                if p.owner == owner && token_matches {
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
pub fn result_owner(v: &serde_json::Value) -> Option<String> {
    if v["type"] != "files_result" {
        return None;
    }
    owner(v["window"].as_str()?)
}
pub fn begin(page: &str, connection: &str) -> io::Result<Ticket> {
    registry().lock().unwrap().begin(page, connection)
}
pub fn publish(ticket: &Ticket, bytes: Vec<u8>, mime: &str) -> io::Result<String> {
    let bytes: Arc<[u8]> = bytes.into(); // Allocate before acquiring the shared registry.
    registry().lock().unwrap().publish(ticket, bytes, mime)
}
pub fn revoke_page(page: &str) {
    registry().lock().unwrap().pages.remove(page);
}
pub fn revoke_owner(owner: &str) {
    registry().lock().unwrap().revoke_owner(owner);
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
    let snapshot = { registry().lock().unwrap().resolve(owner, req) };
    snapshot.into_response() // Copy only after the global mutex has been released.
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
        let url = r
            .publish(&t, (0..10).collect::<Vec<u8>>(), "video/mp4")
            .unwrap();
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
            assert_eq!(head.status(), 200);
            assert_eq!(head.headers()["Content-Length"], "10");
        }
        for bad in [
            "bytes=10-",
            "bytes=-0",
            "bytes=999999999999999999999999999999-",
        ] {
            let resp = r.response("w", &req(&url, Some(bad), "GET"));
            assert_eq!(resp.status(), 416);
            assert_eq!(resp.headers()["Content-Range"], "bytes */10");
        }
        for ignored in ["bytes=5-2", "bytes=0-1,4-5", "garbage", "bytes=+1-2"] {
            let resp = r.response("w", &req(&url, Some(ignored), "GET"));
            assert_eq!(resp.status(), 200);
            assert_eq!(resp.body().len(), 10);
        }
        for huge in [
            "bytes=0-18446744073709551615",
            "bytes=0-9999999999999999999999999999999999999999",
            "bytes=-9999999999999999999999999999999999",
        ] {
            let resp = r.response("w", &req(&url, Some(huge), "GET"));
            assert_eq!(resp.status(), 206);
            assert_eq!(resp.body().len(), 10);
        }
        assert_eq!(r.response("w", &req(&url, None, "POST")).status(), 405);
        let conditional = Request::builder()
            .uri(&url)
            .header("Range", "bytes=1-2")
            .header("If-Range", "\"unmatched\"")
            .body(Vec::new())
            .unwrap();
        assert_eq!(r.response("w", &conditional).status(), 200);
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
    }
    #[test]
    fn retained_payloads_are_bounded_without_breaking_another_windows_player() {
        let mut r = Registry::default();
        let mut urls = Vec::new();
        for i in 0..4 {
            let p = format!("page-{i}");
            let w = format!("window-{i}");
            r.bind(&p, &w);
            let t = r.begin(&p, "c").unwrap();
            urls.push(r.publish(&t, vec![0; MAX_ITEM], "video/mp4").unwrap());
        }
        r.bind("page-4", "window-4");
        let t = r.begin("page-4", "c").unwrap();
        assert!(r.publish(&t, vec![0; MAX_ITEM], "image/png").is_err());
        assert_eq!(
            r.pages
                .values()
                .filter_map(|p| p.asset.as_ref())
                .map(|a| a.bytes.len())
                .sum::<usize>(),
            MAX_BYTES
        );
        let first = r.response("window-0", &req(&urls[0], Some("bytes=0-1"), "GET"));
        assert_eq!(first.status(), 206);
        assert_eq!(first.body().as_ref(), &[0, 0]);
        r.revoke_owner("window-0"); // Native close works even without a FILE_PAGES entry.
        let url = r.publish(&t, vec![0; MAX_ITEM], "image/png").unwrap();
        let last = r.response("window-4", &req(&url, None, "HEAD"));
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
        assert_eq!(response(&owner, &req(&url, None, "HEAD")).status(), 200);
        assert!(!admit_ipc(&release, &owner));
        assert_eq!(response(&owner, &req(&url, None, "HEAD")).status(), 404);
        assert!(publish(&t, vec![42], "image/png").is_err());
        revoke_owner(&owner);
    }
    #[test]
    fn images_detach_before_copy_but_players_keep_future_ranges() {
        let mut r = Registry::default();
        r.bind("image-page", "native-window");
        let t = r.begin("image-page", "c").unwrap();
        let url = r.publish(&t, vec![1, 2, 3], "image/png").unwrap();
        assert_eq!(
            r.response("native-window", &req(&url, None, "HEAD"))
                .status(),
            200
        );
        assert_eq!(
            r.response("native-window", &req(&url, Some("bytes=0-0"), "GET"))
                .status(),
            206
        );
        let snapshot = r.resolve("native-window", &req(&url, None, "GET"));
        assert!(
            r.pages["image-page"].asset.is_none(),
            "image pinned while full response copy is deferred"
        );
        assert_eq!(snapshot.into_response().body().as_ref(), &[1, 2, 3]);
        assert_eq!(
            r.response("native-window", &req(&url, None, "GET"))
                .status(),
            404
        );
        let video = r.publish(&t, vec![1, 2, 3], "video/mp4").unwrap();
        assert_eq!(
            r.response("native-window", &req(&video, None, "GET"))
                .status(),
            200
        );
        assert_eq!(
            r.response("native-window", &req(&video, Some("bytes=1-2"), "GET"))
                .body()
                .as_ref(),
            &[2, 3]
        );
        r.revoke_owner("native-window");
        assert!(!r.pages.contains_key("image-page"));
        assert!(r.publish(&t, vec![1], "image/png").is_err());
        assert_eq!(
            r.response("native-window", &req(&video, None, "GET"))
                .status(),
            404
        );
    }
    #[test]
    fn stale_image_load_and_foreign_page_release_do_not_revoke_a_new_player() {
        let page = random().unwrap();
        let native = random().unwrap();
        assert!(bind_page(&page, &native));
        let first = begin(&page, "c").unwrap();
        let image = publish(&first, vec![1], "image/png").unwrap();
        let second = begin(&page, "c").unwrap();
        let video = publish(&second, vec![2], "video/mp4").unwrap();
        assert!(!admit_ipc(
            &serde_json::json!({"cmd":"files_preview_release","page":page,"url":image}),
            &native
        ));
        assert_eq!(response(&native, &req(&video, None, "GET")).status(), 200);
        let release = serde_json::json!({"cmd":"files_release_page","page":page});
        assert!(!admit_ipc(&release, "foreign"));
        assert!(admit_ipc(&release, &native));
        assert_eq!(owner(&page).as_deref(), Some(native.as_str()));
        revoke_owner(&native);
        assert!(owner(&page).is_none());
    }
    #[test]
    fn same_device_results_route_only_to_the_live_native_page() {
        let (a, b) = (random().unwrap(), random().unwrap());
        let (wa, wb) = (random().unwrap(), random().unwrap());
        assert!(bind_page(&a, &wa));
        assert!(bind_page(&b, &wb));
        let event = serde_json::json!({"type":"files_result","id":"same-device","window":b});
        assert_eq!(result_owner(&event).as_deref(), Some(wb.as_str()));
        assert!(result_owner(&serde_json::json!({"type":"files_job","window":b})).is_none());
        revoke_owner(&wb);
        assert!(result_owner(&event).is_none());
        assert_eq!(
            result_owner(&serde_json::json!({"type":"files_result","id":"same-device","window":a}))
                .as_deref(),
            Some(wa.as_str())
        );
        revoke_owner(&wa);
    }
}
