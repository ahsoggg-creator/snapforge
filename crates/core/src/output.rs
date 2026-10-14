//! Getting an image out: encode, save to the screenshots folder / a chosen path, copy to the clipboard.

use std::fs;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, RgbaImage};

use crate::history::encode_png_fast;
use crate::naming;
use crate::settings::ImageFormat;

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
    #[error("clipboard: {0}")]
    Clipboard(String),
}

pub fn encode(img: &RgbaImage, format: ImageFormat, jpeg_quality: u8) -> Result<Vec<u8>, OutputError> {
    match format {
        ImageFormat::Png => Ok(encode_png_fast(img)?),
        ImageFormat::Jpeg => {
            // JPEG has no alpha channel
            let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
            let mut out = Vec::new();
            JpegEncoder::new_with_quality(&mut out, jpeg_quality.clamp(1, 100)).encode_image(&rgb)?;
            Ok(out)
        }
    }
}

/// Format by file extension (`.jpg`/`.jpeg` → JPEG, everything else → PNG).
pub fn format_for_path(path: &Path) -> ImageFormat {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => ImageFormat::Jpeg,
        _ => ImageFormat::Png,
    }
}

/// Writes `img` to exactly `path` (parent folders are created).
pub fn save_as(img: &RgbaImage, path: &Path, jpeg_quality: u8) -> Result<(), OutputError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, encode(img, format_for_path(path), jpeg_quality)?)?;
    Ok(())
}

/// Writes `dir/stem.ext` (a numeric suffix is added if it exists). Returns the final path.
pub fn save_to_dir(img: &RgbaImage, dir: &Path, stem: &str, format: ImageFormat, jpeg_quality: u8) -> Result<PathBuf, OutputError> {
    fs::create_dir_all(dir)?;
    let path = naming::unique_path(dir, &naming::sanitize(stem), format.ext());
    fs::write(&path, encode(img, format, jpeg_quality)?)?;
    Ok(path)
}

pub fn copy_to_clipboard(img: &RgbaImage) -> Result<(), OutputError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| OutputError::Clipboard(e.to_string()))?;
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: std::borrow::Cow::Borrowed(img.as_raw()),
    })
    .map_err(|e| OutputError::Clipboard(e.to_string()))
}

pub fn copy_text_to_clipboard(text: &str) -> Result<(), OutputError> {
    let mut cb = arboard::Clipboard::new().map_err(|e| OutputError::Clipboard(e.to_string()))?;
    cb.set_text(text.to_owned()).map_err(|e| OutputError::Clipboard(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn img() -> RgbaImage {
        RgbaImage::from_fn(32, 16, |x, y| Rgba([(x * 8) as u8, (y * 16) as u8, 90, 255]))
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sf-out-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn png_roundtrip_is_lossless() {
        let bytes = encode(&img(), ImageFormat::Png, 90).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8(), img());
    }

    #[test]
    fn jpeg_encodes_without_alpha() {
        let bytes = encode(&img(), ImageFormat::Jpeg, 80).unwrap();
        let back = image::load_from_memory(&bytes).unwrap();
        assert_eq!((back.width(), back.height()), (32, 16));
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(format_for_path(Path::new("a.JPG")), ImageFormat::Jpeg);
        assert_eq!(format_for_path(Path::new("a.png")), ImageFormat::Png);
        assert_eq!(format_for_path(Path::new("noext")), ImageFormat::Png);
    }

    #[test]
    fn save_to_dir_never_overwrites() {
        let d = tmp("dir");
        let a = save_to_dir(&img(), &d, "shot", ImageFormat::Png, 90).unwrap();
        let b = save_to_dir(&img(), &d, "shot", ImageFormat::Png, 90).unwrap();
        assert_ne!(a, b);
        assert!(a.exists() && b.exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_as_creates_parent_folders() {
        let d = tmp("as");
        let p = d.join("a").join("b.png");
        save_as(&img(), &p, 90).unwrap();
        assert!(p.exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
