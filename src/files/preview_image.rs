//! Portable bounded thumbnails; animations and formats without a bounded decoder stay intact.
#[cfg(all(not(test), panic = "abort"))]
compile_error!("Preview decoder recovery requires panic=unwind in application builds");
use super::preview_cache::Prepared;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};
use std::{
    cell::Cell,
    io::{self, Cursor},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};
const EDGE: u32 = 1600;
const DECODED: u64 = 128 * 1024 * 1024;
static DECODE: Mutex<()> = Mutex::new(());
static FETCHES: Mutex<usize> = Mutex::new(0);
static FETCH_READY: Condvar = Condvar::new();
thread_local! { static HANDLED_DECODE: Cell<bool> = const { Cell::new(false) }; }
#[cfg(any(windows, test))]
pub(super) fn panic_is_handled() -> bool {
    HANDLED_DECODE.try_with(Cell::get).unwrap_or(false)
}
struct HandledDecode(bool);
impl HandledDecode {
    fn enter() -> Self {
        Self(HANDLED_DECODE.with(|v| v.replace(true)))
    }
}
impl Drop for HandledDecode {
    fn drop(&mut self) {
        HANDLED_DECODE.with(|v| v.set(self.0));
    }
}
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

pub fn prepare(
    bytes: Vec<u8>,
    mime: &str,
    ticket: Option<&super::preview::Ticket>,
) -> io::Result<Prepared> {
    let unchanged = |bytes| Prepared {
        bytes: Arc::new(bytes),
        mime: mime.into(),
        first_page: false,
    };
    let format = match mime {
        "image/png" if !animated_png(&bytes) => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/bmp" => ImageFormat::Bmp,
        "image/webp"
            if !(bytes.get(..4) == Some(b"RIFF")
                && bytes.get(8..12) == Some(b"WEBP")
                && bytes.get(12..16) == Some(b"VP8X")
                && bytes.get(20).is_some_and(|b| b & 2 != 0)) =>
        {
            ImageFormat::WebP
        }
        _ => return Ok(unchanged(bytes)),
    };
    // One thumbnail decode at a time, separate from the SFTP/browse locks.
    decode_safely(&DECODE, || {
        if ticket.is_some_and(|t| !super::preview::current(t)) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "이전 미리보기입니다",
            ));
        }
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
    })
}

fn decode_safely<T>(mutex: &Mutex<()>, decode: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    catch_unwind(AssertUnwindSafe(|| {
        let _handled = HandledDecode::enter();
        let _guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
        decode()
    }))
    .unwrap_or_else(|_| {
        Err(io::Error::other(
            "이미지 디코더에서 오류가 발생했습니다. 다른 파일을 선택하거나 다시 시도해 주세요",
        ))
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
    #[test]
    fn big_image_is_reduced_and_small_image_is_byte_preserved() {
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(2048, 1024)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let result = prepare(png.into_inner(), "image/png", None).unwrap();
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
        assert!(prepare(vec![1, 2, 3], "image/png", None).is_err());
        assert!(prepare(b"not a PNG with acTL text".to_vec(), "image/png", None).is_err());
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
        let result = prepare(bytes, "image/jpeg", None).unwrap();
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
    fn decoder_panic_is_an_error_and_poison_does_not_disable_the_next_request() {
        let lock = Mutex::new(());
        assert!(!panic_is_handled());
        assert!(decode_safely::<()>(&lock, || {
            assert!(panic_is_handled());
            panic!("generated decoder panic")
        })
        .is_err());
        assert!(
            !panic_is_handled(),
            "panic handling scope leaked after recovery"
        );
        assert!(lock.is_poisoned());
        assert_eq!(decode_safely(&lock, || Ok(7)).unwrap(), 7);
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(8, 4)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        assert!(prepare(png.into_inner(), "image/png", None).is_ok());
    }
    #[test]
    fn opaque_thumbnails_use_jpeg_and_transparent_ones_keep_png() {
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(2048, 1024)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let opaque = prepare(png.into_inner(), "image/png", None).unwrap();
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
        assert!(catch_unwind(AssertUnwindSafe(|| {
            let _guard = counter.lock().unwrap();
            panic!("generated counter poison");
        }))
        .is_err());
        *fetch_lock(&counter) += 1;
        *fetch_lock(&counter) -= 1;
        assert_eq!(*fetch_lock(&counter), 1);
    }
}
