//! Portable bounded thumbnails; animations and formats without a bounded decoder stay intact.
use super::preview_cache::Prepared;
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader};
use std::{
    io::{self, Cursor},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
const EDGE: u32 = 1600;
const DECODED: u64 = 128 * 1024 * 1024;
static DECODE: Mutex<()> = Mutex::new(());
static FETCHES: Mutex<usize> = Mutex::new(0);
static FETCH_READY: Condvar = Condvar::new();
/// Limit downloaded originals waiting for the one decoder, across all windows.
pub struct Work;
impl Drop for Work {
    fn drop(&mut self) {
        *FETCHES.lock().unwrap() -= 1;
        FETCH_READY.notify_one();
    }
}
pub fn work(ticket: Option<&super::preview::Ticket>) -> io::Result<Work> {
    let by = Instant::now() + Duration::from_secs(30);
    let mut n = FETCHES.lock().unwrap();
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
            .unwrap()
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
        "image/png" if !bytes.windows(4).any(|b| b == b"acTL") => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/bmp" => ImageFormat::Bmp,
        "image/webp"
            if !(bytes.get(12..16) == Some(b"VP8X")
                && bytes.get(20).is_some_and(|b| b & 2 != 0)) =>
        {
            ImageFormat::WebP
        }
        _ => return Ok(unchanged(bytes)),
    };
    // One thumbnail decode at a time, separate from the SFTP/browse locks.
    let _guard = DECODE.lock().unwrap();
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
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(failed)?;
    decoded.apply_orientation(orientation);
    let thumbnail = decoded.thumbnail(EDGE, EDGE);
    let mut out = Cursor::new(Vec::new());
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
    if out.get_ref().len() > 16 * 1024 * 1024 {
        return Err(io::Error::other("이미지 미리보기 크기 제한을 넘었습니다"));
    }
    Ok(Prepared {
        bytes: Arc::new(out.into_inner()),
        mime: "image/png".into(),
        first_page: false,
    })
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
        let dimensions =
            ImageReader::with_format(Cursor::new(result.bytes.as_slice()), ImageFormat::Png)
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
            ImageReader::with_format(Cursor::new(result.bytes.as_slice()), ImageFormat::Png)
                .into_dimensions()
                .unwrap(),
            (800, 1600)
        );
    }
}
