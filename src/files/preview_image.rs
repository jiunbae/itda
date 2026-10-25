//! Bounded thumbnails decoded only in a disposable child; app panic strategy stays abort.
use super::preview_cache::Prepared;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};
use std::{
    io::{self, Cursor, Read, Write},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, TryLockError},
    time::{Duration, Instant},
};
const EDGE: u32 = 1600;
const DECODED: u64 = 128 * 1024 * 1024;
const PAYLOAD: usize = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const MAGIC: &[u8] = b"ITDP1";
static DECODE: Mutex<()> = Mutex::new(());
static FETCHES: Mutex<usize> = Mutex::new(0);
static FETCH_READY: Condvar = Condvar::new();
fn fetch_lock(mutex: &Mutex<usize>) -> MutexGuard<'_, usize> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}
/// Limit downloaded originals waiting for the one decoder, across all windows.
pub struct Work;
impl Drop for Work {
    fn drop(&mut self) {
        *fetch_lock(&FETCHES) -= 1;
        FETCH_READY.notify_one();
    }
}
pub fn work(ticket: Option<&super::preview::Ticket>) -> io::Result<Work> {
    let by = Instant::now() + Duration::from_secs(30);
    let mut n = fetch_lock(&FETCHES);
    loop {
        if ticket.is_some_and(|t| !super::preview::current(t)) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "이전 미리보기입니다",
            ));
        }
        if *n < 4 {
            *n += 1;
            return Ok(Work);
        }
        if Instant::now() >= by {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "다른 미리보기를 처리 중입니다. 다시 선택해 주세요",
            ));
        }
        n = FETCH_READY
            .wait_timeout(n, Duration::from_millis(50))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

fn unchanged(bytes: Vec<u8>, mime: &str) -> Prepared {
    Prepared {
        bytes: Arc::new(bytes),
        mime: mime.into(),
        first_page: false,
    }
}
// Minimal, bounds-checked dimension fields only; no codec construction in the parent.
fn dimensions(b: &[u8], mime: &str) -> Option<(u32, u32)> {
    let le32 = |at: usize| {
        Some(u32::from_le_bytes(
            b.get(at..at.checked_add(4)?)?.try_into().ok()?,
        ))
    };
    match mime {
        "image/png"
            if b.get(..8) == Some(b"\x89PNG\r\n\x1a\n")
                && b.get(8..16) == Some(b"\0\0\0\rIHDR") =>
        {
            Some((
                u32::from_be_bytes(b.get(16..20)?.try_into().ok()?),
                u32::from_be_bytes(b.get(20..24)?.try_into().ok()?),
            ))
        }
        "image/bmp" if b.get(..2) == Some(b"BM") => match le32(14)? {
            12 => Some((
                u16::from_le_bytes(b.get(18..20)?.try_into().ok()?) as u32,
                u16::from_le_bytes(b.get(20..22)?.try_into().ok()?) as u32,
            )),
            n if n >= 40 => Some((
                (le32(18)? as i32).unsigned_abs(),
                (le32(22)? as i32).unsigned_abs(),
            )),
            _ => None,
        },
        "image/webp" if b.get(..4) == Some(b"RIFF") && b.get(8..12) == Some(b"WEBP") => {
            let le24 = |at: usize| {
                let s = b.get(at..at.checked_add(3)?)?;
                Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
            };
            match b.get(12..16)? {
                b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
                b"VP8 " if b.get(23..26) == Some(b"\x9d\x01\x2a") => Some((
                    u16::from_le_bytes(b.get(26..28)?.try_into().ok()?) as u32 & 0x3fff,
                    u16::from_le_bytes(b.get(28..30)?.try_into().ok()?) as u32 & 0x3fff,
                )),
                b"VP8L" if b.get(20) == Some(&0x2f) => {
                    let bits = le32(21)?;
                    Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
                }
                _ => None,
            }
        }
        "image/jpeg" if b.get(..2) == Some(b"\xff\xd8") => {
            let mut at = 2usize;
            loop {
                if *b.get(at)? != 0xff {
                    return None;
                }
                while *b.get(at)? == 0xff {
                    at += 1;
                }
                let marker = *b.get(at)?;
                at += 1;
                if matches!(marker, 0xd9 | 0xda | 0x00) {
                    return None;
                }
                if matches!(marker, 0x01 | 0xd0..=0xd8) {
                    continue;
                }
                let len =
                    u16::from_be_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?) as usize;
                if len < 2 {
                    return None;
                }
                let segment = b.get(at..at.checked_add(len)?)?;
                if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                    return Some((
                        u16::from_be_bytes(segment.get(5..7)?.try_into().ok()?) as u32,
                        u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?) as u32,
                    ));
                }
                at += len;
            }
        }
        _ => None,
    }
}
fn format(mime: &str) -> Option<(u8, ImageFormat)> {
    Some(match mime {
        "image/png" => (1, ImageFormat::Png),
        "image/jpeg" => (2, ImageFormat::Jpeg),
        "image/bmp" => (3, ImageFormat::Bmp),
        "image/webp" => (4, ImageFormat::WebP),
        _ => return None,
    })
}
fn mime(code: u8) -> io::Result<&'static str> {
    match code {
        1 => Ok("image/png"),
        2 => Ok("image/jpeg"),
        3 => Ok("image/bmp"),
        4 => Ok("image/webp"),
        _ => Err(io::Error::other("잘못된 이미지 미리보기 응답입니다")),
    }
}
fn preserved(bytes: &[u8], mime: &str) -> bool {
    format(mime).is_none()
        || (mime == "image/png" && animated_png(bytes))
        || (mime == "image/webp"
            && bytes.get(..4) == Some(b"RIFF")
            && bytes.get(8..12) == Some(b"WEBP")
            && bytes.get(12..16) == Some(b"VP8X")
            && bytes.get(20).is_some_and(|b| b & 2 != 0))
}
fn canceled(ticket: Option<&super::preview::Ticket>) -> bool {
    ticket.is_some_and(|t| !super::preview::current(t))
}
fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "이전 미리보기입니다")
}
fn timeout() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "이미지 미리보기 시간이 초과되었습니다",
    )
}

pub fn prepare(
    bytes: Vec<u8>,
    mime: &str,
    ticket: Option<&super::preview::Ticket>,
) -> io::Result<Prepared> {
    if canceled(ticket) {
        return Err(interrupted());
    }
    if bytes.len() > PAYLOAD {
        return Err(io::Error::other("이미지 미리보기 크기 제한을 넘었습니다"));
    }
    // Header-only inspection uses no image decoder inside the application process.
    if preserved(&bytes, mime)
        || dimensions(&bytes, mime).is_some_and(|(w, h)| w > 0 && h > 0 && w <= EDGE && h <= EDGE)
    {
        return Ok(unchanged(bytes, mime));
    }
    let started = Instant::now();
    let _guard = loop {
        if canceled(ticket) {
            return Err(interrupted());
        }
        if started.elapsed() >= TIMEOUT {
            return Err(timeout());
        }
        match DECODE.try_lock() {
            Ok(g) => break g,
            Err(TryLockError::Poisoned(e)) => break e.into_inner(),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--preview-decode");
    let mut input = Vec::with_capacity(bytes.len() + 1);
    input.push(
        format(mime)
            .ok_or_else(|| io::Error::other("지원하지 않는 이미지입니다"))?
            .0,
    );
    input.extend_from_slice(&bytes);
    let output = run_worker(command, input, || canceled(ticket), TIMEOUT)?;
    if !output.starts_with(MAGIC) {
        return Err(io::Error::other("잘못된 이미지 미리보기 응답입니다"));
    }
    let output_mime = mime_from_response(&output)?;
    Ok(unchanged(output[MAGIC.len() + 1..].to_vec(), output_mime))
}
fn mime_from_response(output: &[u8]) -> io::Result<&'static str> {
    if output.len() <= MAGIC.len() + 1 {
        return Err(io::Error::other("빈 이미지 미리보기 응답입니다"));
    }
    mime(output[MAGIC.len()])
}

// Always kill AND reap on early return. Pipe readers/writers never own a child handle.
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn run_worker(
    mut command: Command,
    input: Vec<u8>,
    canceled: impl Fn() -> bool,
    limit: Duration,
) -> io::Result<Vec<u8>> {
    if canceled() {
        return Err(interrupted());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW; never flash a console.
    }
    let mut worker = Worker(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let mut stdin = worker
        .0
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("미리보기 입력 파이프 오류"))?;
    let stdout = worker
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("미리보기 출력 파이프 오류"))?;
    let (tx, rx) = mpsc::channel();
    let writer = std::thread::Builder::new()
        .name("preview-input".into())
        .spawn(move || stdin.write_all(&input))?;
    let reader = std::thread::Builder::new()
        .name("preview-output".into())
        .spawn(move || {
            let mut output = Vec::new();
            let result = stdout
                .take((PAYLOAD + MAGIC.len() + 2) as u64)
                .read_to_end(&mut output)
                .and_then(|_| {
                    if output.len() > PAYLOAD + MAGIC.len() + 1 {
                        Err(io::Error::other("이미지 미리보기 응답이 너무 큽니다"))
                    } else {
                        Ok(output)
                    }
                });
            let _ = tx.send(result);
        });
    let reader = match reader {
        Ok(r) => r,
        Err(e) => {
            drop(worker);
            let _ = writer.join();
            return Err(e);
        }
    };
    let started = Instant::now();
    let result = (|| {
        loop {
            if canceled() {
                return Err(interrupted());
            }
            if started.elapsed() >= limit {
                return Err(timeout());
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(output) => {
                    let output = output?;
                    // EOF is insufficient: wait for successful exit under the same deadline.
                    loop {
                        if canceled() {
                            return Err(interrupted());
                        }
                        if started.elapsed() >= limit {
                            return Err(timeout());
                        }
                        if let Some(status) = worker.0.try_wait()? {
                            return if status.success() {
                                Ok(output)
                            } else {
                                Err(io::Error::other(
                                    "이미지 미리보기 프로세스가 종료되었습니다",
                                ))
                            };
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(_) => return Err(io::Error::other("이미지 미리보기 응답을 받지 못했습니다")),
            }
        }
    })();
    drop(worker); // Close the process before joining potentially blocked pipe IO.
    let written = writer
        .join()
        .map_err(|_| io::Error::other("미리보기 입력 오류"));
    let _ = reader.join();
    let output = result?;
    written??;
    Ok(output)
}

/// Called before askpass, configuration, logging, panic hooks or GUI initialization.
pub fn decode_mode() -> bool {
    if std::env::args().nth(1).as_deref() != Some("--preview-decode") {
        return false;
    }
    let result = (|| -> io::Result<()> {
        let mut input = Vec::new();
        io::stdin()
            .take((PAYLOAD + 2) as u64)
            .read_to_end(&mut input)?;
        if input.len() < 2 || input.len() > PAYLOAD + 1 {
            return Err(io::Error::other("invalid preview input"));
        }
        let mime = mime(input[0])?;
        let output = decode(input[1..].to_vec(), mime)?;
        let code = format(&output.mime)
            .ok_or_else(|| io::Error::other("invalid preview output"))?
            .0;
        let mut stdout = io::stdout().lock();
        stdout.write_all(MAGIC)?;
        stdout.write_all(&[code])?;
        stdout.write_all(&output.bytes)?;
        stdout.flush()
    })();
    if result.is_err() {
        std::process::exit(2);
    }
    true
}

// This function is reachable from production only through decode_mode in the child.
fn decode(bytes: Vec<u8>, mime: &str) -> io::Result<Prepared> {
    let format = format(mime)
        .ok_or_else(|| io::Error::other("unsupported image"))?
        .1;
    let unchanged = |bytes| unchanged(bytes, mime);
    let failed = |e| io::Error::other(format!("이미지 미리보기를 만들지 못했습니다: {e}"));
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODED);
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(failed)?;
    let (w, h) = decoder.dimensions();
    if decoder.total_bytes() > DECODED || w == 0 || h == 0 {
        return Err(io::Error::other(
            "이미지 해상도가 미리보기 제한을 넘었습니다",
        ));
    }
    if w <= EDGE && h <= EDGE {
        drop(decoder);
        return Ok(unchanged(bytes));
    }
    let orientation = decoder.orientation().map_err(failed)?;
    let profile = decoder.icc_profile().map_err(failed)?;
    let decoded = DynamicImage::from_decoder(decoder).map_err(failed)?;
    let mut thumbnail = decoded.thumbnail(EDGE, EDGE);
    drop(decoded);
    // Rotate only the bounded thumbnail, not a second full-resolution allocation.
    thumbnail.apply_orientation(orientation);
    let mut out = Cursor::new(Vec::new());
    // JPEG/CMYK decoders output RGB. Only an RGB profile belongs on the encoded result.
    let profile = profile.filter(|p| p.get(16..20) == Some(b"RGB "));
    let opaque =
        !thumbnail.color().has_alpha() || thumbnail.to_rgba8().pixels().all(|p| p[3] == 255);
    let output_mime = if opaque {
        let rgb = thumbnail.to_rgb8();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
        if let Some(profile) = profile {
            encoder
                .set_icc_profile(profile)
                .map_err(|e| failed(image::ImageError::Unsupported(e)))?;
        }
        encoder
            .encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(failed)?;
        "image/jpeg"
    } else {
        let rgba = thumbnail.to_rgba8();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
        if let Some(profile) = profile {
            encoder
                .set_icc_profile(profile)
                .map_err(|e| failed(image::ImageError::Unsupported(e)))?;
        }
        encoder
            .write_image(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(failed)?;
        "image/png"
    };
    if out.get_ref().len() > 16 * 1024 * 1024 {
        return Err(io::Error::other("이미지 미리보기 크기 제한을 넘었습니다"));
    }
    Ok(Prepared {
        bytes: Arc::new(out.into_inner()),
        mime: output_mime.into(),
        first_page: false,
    })
}

fn animated_png(bytes: &[u8]) -> bool {
    if bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let mut at = 8usize;
    while let Some(header) = at.checked_add(8).and_then(|end| bytes.get(at..end)) {
        let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let Some(end) = at
            .checked_add(12)
            .and_then(|n| n.checked_add(len))
            .filter(|end| *end <= bytes.len())
        else {
            return false;
        };
        if &header[4..] == b"acTL" {
            return len == 8;
        }
        if &header[4..] == b"IDAT" || &header[4..] == b"IEND" {
            return false;
        }
        at = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "files::preview_image::tests::process_fixture",
            "--nocapture",
        ])
        .env("ITDA_PREVIEW_TEST_CHILD", mode);
        cmd
    }
    #[test]
    fn process_fixture() {
        let Ok(mode) = std::env::var("ITDA_PREVIEW_TEST_CHILD") else {
            return;
        };
        match mode.as_str() {
            "exit" => std::process::exit(23),
            "hang" => std::thread::sleep(Duration::from_secs(30)),
            "overflow" => {
                let _ = io::stdout().write_all(&vec![0; PAYLOAD + 32]);
            }
            "echo" => {
                let mut data = Vec::new();
                io::stdin().read_to_end(&mut data).unwrap();
                io::stdout().write_all(&data).unwrap();
            }
            _ => panic!("unexpected private test mode"),
        }
        std::process::exit(0);
    }
    #[test]
    fn child_exit_timeout_cancel_and_output_limit_do_not_disable_next_request() {
        let started = Instant::now();
        assert!(run_worker(fixture("exit"), vec![1; 256 * 1024], || false, TIMEOUT).is_err());
        assert_eq!(
            run_worker(
                fixture("hang"),
                vec![1; 256 * 1024],
                || false,
                Duration::from_millis(200)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::TimedOut
        );
        let cancel_at = Instant::now();
        assert_eq!(
            run_worker(
                fixture("hang"),
                vec![1; 256 * 1024],
                || cancel_at.elapsed() > Duration::from_millis(100),
                TIMEOUT
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::Interrupted
        );
        assert!(run_worker(fixture("overflow"), vec![], || false, TIMEOUT).is_err());
        // Large simultaneous input/output exercises the full pipes in both directions.
        let input = vec![0x55; 512 * 1024];
        let output = run_worker(fixture("echo"), input.clone(), || false, TIMEOUT).unwrap();
        assert!(output.ends_with(&input));
        assert!(started.elapsed() < Duration::from_secs(10));
    }
    #[test]
    fn header_only_dimensions_and_truncated_headers() {
        for (mime, format) in [
            ("image/png", ImageFormat::Png),
            ("image/jpeg", ImageFormat::Jpeg),
            ("image/bmp", ImageFormat::Bmp),
            ("image/webp", ImageFormat::WebP),
        ] {
            let mut out = Cursor::new(Vec::new());
            DynamicImage::new_rgb8(32, 16)
                .write_to(&mut out, format)
                .unwrap();
            let bytes = out.into_inner();
            assert_eq!(dimensions(&bytes, mime), Some((32, 16)), "{mime}");
            assert_eq!(
                prepare(bytes.clone(), mime, None).unwrap().bytes.as_slice(),
                bytes
            );
            for len in 0..bytes.len().min(100) {
                let _ = dimensions(&bytes[..len], mime);
            }
        }
        assert!(mime_from_response(MAGIC).is_err());
        assert!(mime_from_response(b"ITDP1\xffx").is_err());
        assert!(prepare(vec![0; PAYLOAD + 1], "image/png", None).is_err());
    }
    #[test]
    fn big_image_is_reduced_and_small_image_is_byte_preserved() {
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(2048, 1024)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let result = decode(png.into_inner(), "image/png").unwrap();
        let dimensions = ImageReader::with_format(
            Cursor::new(result.bytes.as_slice()),
            ImageFormat::from_mime_type(&result.mime).unwrap(),
        )
        .into_dimensions()
        .unwrap();
        assert_eq!(dimensions, (1600, 800));
        assert_eq!(result.mime, "image/png");
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(8, 4)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let bytes = png.into_inner();
        assert_eq!(
            prepare(bytes.clone(), "image/png", None)
                .unwrap()
                .bytes
                .as_slice(),
            bytes
        );
    }
    #[test]
    fn animations_and_media_are_preserved_and_bad_image_fails() {
        for mime in ["image/gif", "image/avif", "video/mp4", "audio/mpeg"] {
            assert_eq!(
                prepare(vec![1, 2, 3], mime, None).unwrap().bytes.as_slice(),
                &[1, 2, 3]
            );
        }
        assert!(decode(vec![1, 2, 3], "image/png").is_err());
        assert!(decode(b"not a PNG with acTL text".to_vec(), "image/png").is_err());
        let mut text = b"\x89PNG\r\n\x1a\n".to_vec();
        text.extend_from_slice(&4u32.to_be_bytes());
        text.extend_from_slice(b"tEXtacTL0000");
        assert!(
            !animated_png(&text),
            "chunk payload text is not an animation control chunk"
        );
    }
    #[test]
    fn jpeg_orientation_is_applied_before_thumbnailing() {
        let mut jpeg = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(2048, 1024)
            .write_to(&mut jpeg, ImageFormat::Jpeg)
            .unwrap();
        let original = jpeg.into_inner();
        // Little-endian EXIF orientation=6 (rotate 90 degrees clockwise).
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut bytes = original[..2].to_vec();
        bytes.extend_from_slice(&[0xff, 0xe1]);
        bytes.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        bytes.extend_from_slice(exif);
        bytes.extend_from_slice(&original[2..]);
        let result = decode(bytes, "image/jpeg").unwrap();
        assert_eq!(
            ImageReader::with_format(
                Cursor::new(result.bytes.as_slice()),
                ImageFormat::from_mime_type(&result.mime).unwrap()
            )
            .into_dimensions()
            .unwrap(),
            (800, 1600)
        );
    }
    #[test]
    fn opaque_thumbnails_use_jpeg_and_transparent_ones_keep_png() {
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(2048, 1024)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let opaque = decode(png.into_inner(), "image/png").unwrap();
        assert_eq!(opaque.mime, "image/jpeg");
        assert_eq!(
            ImageReader::with_format(Cursor::new(opaque.bytes.as_slice()), ImageFormat::Jpeg)
                .into_dimensions()
                .unwrap(),
            (1600, 800)
        );
    }
    #[test]
    fn poisoned_fetch_counter_still_admits_and_releases_work() {
        let counter = Mutex::new(1usize);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = counter.lock().unwrap();
            panic!("generated counter poison");
        }))
        .is_err());
        *fetch_lock(&counter) += 1;
        *fetch_lock(&counter) -= 1;
        assert_eq!(*fetch_lock(&counter), 1);
    }
}
