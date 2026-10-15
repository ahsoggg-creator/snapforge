//! Offline text recognition through `Windows.Media.Ocr`. No cloud: it uses the OCR languages installed in Windows.

use image::{RgbaImage, imageops};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

use crate::geometry::Rect;
use crate::sensitive::{OcrData, OcrLine, OcrWord};

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error(
        "no OCR language is installed in Windows (Settings → Time & language → Language → add a language pack with OCR)"
    )]
    NoEngine,
    #[error("OCR language '{0}' is not installed")]
    LanguageMissing(String),
    #[error("OCR failed: {0}")]
    Failed(String),
}

fn failed(e: windows::core::Error) -> OcrError {
    OcrError::Failed(e.message())
}

/// Language tags (`en-US`, `ru`, …) that Windows can recognise on this computer.
pub fn available_languages() -> Vec<String> {
    OcrEngine::AvailableRecognizerLanguages()
        .ok()
        .map(|v| {
            v.into_iter()
                .filter_map(|l| l.LanguageTag().ok().map(|t| t.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn engine(language: Option<&str>) -> Result<OcrEngine, OcrError> {
    match language {
        Some(tag) => {
            let lang = Language::CreateLanguage(&HSTRING::from(tag)).map_err(failed)?;
            if !OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
                return Err(OcrError::LanguageMissing(tag.to_owned()));
            }
            OcrEngine::TryCreateFromLanguage(&lang).map_err(|_| OcrError::LanguageMissing(tag.to_owned()))
        }
        None => OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| OcrError::NoEngine),
    }
}

/// Recognises the text in `img`. `language` is a tag from `available_languages`; `None` uses the user's languages.
/// Rectangles are in pixels of `img`.
///
/// Small interface text is read much better when enlarged, large text is not: so the picture is read at its own
/// size and, if it is small enough, enlarged 2x as well; the pass that recognised more characters wins (ties go to
/// the original size).
pub fn recognize(img: &RgbaImage, language: Option<&str>) -> Result<OcrData, OcrError> {
    let engine = engine(language)?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(64) as f64;
    let longest = img.width().max(img.height()) as f64;
    let base = recognize_at(&engine, img, (1.0_f64).min(max / longest))?;
    if longest * 2.0 > 3000.0_f64.max(longest) || longest * 2.0 > max {
        return Ok(base);
    }
    let big = recognize_at(&engine, img, 2.0)?;
    let score = |d: &OcrData| d.text.chars().filter(|c| c.is_alphanumeric()).count();
    Ok(if score(&big) * 100 > score(&base) * 105 {
        big
    } else {
        base
    })
}

fn recognize_at(engine: &OcrEngine, img: &RgbaImage, k: f64) -> Result<OcrData, OcrError> {
    let scaled;
    let src: &RgbaImage = if (k - 1.0).abs() > f64::EPSILON {
        scaled = imageops::resize(
            img,
            ((img.width() as f64 * k) as u32).max(1),
            ((img.height() as f64 * k) as u32).max(1),
            imageops::FilterType::Lanczos3,
        );
        &scaled
    } else {
        img
    };

    let mut bgra = Vec::with_capacity(src.as_raw().len());
    for p in src.as_raw().chunks_exact(4) {
        bgra.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    let writer = DataWriter::new().map_err(failed)?;
    writer.WriteBytes(&bgra).map_err(failed)?;
    let buffer = writer.DetachBuffer().map_err(failed)?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        src.width() as i32,
        src.height() as i32,
    )
    .map_err(failed)?;
    let result = engine.RecognizeAsync(&bitmap).map_err(failed)?.join().map_err(failed)?;

    let mut lines = Vec::new();
    for line in result.Lines().map_err(failed)? {
        let mut words = Vec::new();
        for w in line.Words().map_err(failed)? {
            let r = w.BoundingRect().map_err(failed)?;
            words.push(OcrWord {
                text: w.Text().map_err(failed)?.to_string(),
                rect: Rect::new(
                    (r.X as f64 / k).floor() as i32,
                    (r.Y as f64 / k).floor() as i32,
                    (r.Width as f64 / k).ceil() as i32,
                    (r.Height as f64 / k).ceil() as i32,
                ),
            });
        }
        lines.push(OcrLine { words });
    }
    Ok(OcrData {
        text: result.Text().map_err(failed)?.to_string(),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Document, Pt, Shape};

    fn text_image(text: &str, size: f32) -> RgbaImage {
        let mut d = Document::new(RgbaImage::from_pixel(900, 160, image::Rgba([255, 255, 255, 255])));
        d.add(
            Shape::Text {
                at: Pt::new(20.0, 40.0),
                text: text.to_owned(),
                size,
            },
            [0, 0, 0, 255],
            1.0,
        );
        d.render()
    }

    #[test]
    fn recognises_rendered_text_offline_with_word_boxes() {
        if available_languages().is_empty() || !crate::editor::render::has_font() {
            eprintln!("no OCR language or system font here: skipped");
            return;
        }
        let img = text_image("Invoice total 4200 EUR", 44.0);
        let data = recognize(&img, None).expect("recognise");
        eprintln!(
            "OCR languages: {:?}; recognised: {:?}",
            available_languages(),
            data.text
        );
        let lower = data.text.to_lowercase();
        assert!(
            lower.contains("invoice") && lower.contains("total"),
            "recognised: {}",
            data.text
        );
        let word = data
            .lines
            .iter()
            .flat_map(|l| &l.words)
            .find(|w| w.text.to_lowercase().contains("invoice"))
            .expect("a word box");
        assert!(
            word.rect.x >= 10 && word.rect.x < 120 && word.rect.y >= 20 && word.rect.y < 120,
            "{:?}",
            word.rect
        );
    }

    #[test]
    fn unknown_language_is_reported() {
        match recognize(&text_image("x", 30.0), Some("xx-ZZ")) {
            Err(OcrError::LanguageMissing(t)) => assert_eq!(t, "xx-ZZ"),
            Err(OcrError::Failed(_)) => {} // invalid tag rejected by Windows itself
            other => panic!("expected an error, got {other:?}"),
        }
    }
}
