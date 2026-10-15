//! Generated pictures for `--demo` and `--screenshots`: invented windows, no real screen content, nothing read from
//! or written to the user's data folders.

use snapforge_core::RgbaImage;
use snapforge_core::editor::{Document, Pt, Shape};
use snapforge_core::image::Rgba;

const THEMES: [([u8; 3], [u8; 3]); 4] = [
    ([32, 52, 96], [88, 60, 150]),
    ([20, 84, 80], [40, 130, 110]),
    ([110, 48, 60], [190, 100, 80]),
    ([40, 40, 52], [90, 96, 120]),
];

const TITLES: [&str; 4] = [
    "Quarterly report — Acme Demo",
    "Notes.txt — Demo Editor",
    "Dashboard — Example Corp",
    "Invoice 0042 — Sample LLC",
];
const APPS: [&str; 4] = ["demo-office", "demo-notes", "demo-browser", "demo-billing"];

pub fn app_name(seed: usize) -> &'static str {
    APPS[seed % APPS.len()]
}

fn px(x: f32, y: f32) -> Pt {
    Pt::new(x, y)
}

/// A fictional application window on a gradient desktop.
pub fn demo_image(seed: usize, w: u32, h: u32) -> RgbaImage {
    let (c0, c1) = THEMES[seed % THEMES.len()];
    let mut img = RgbaImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let t = (x as f32 / w as f32 + y as f32 / h as f32) / 2.0;
        let m = |i: usize| (c0[i] as f32 + (c1[i] as f32 - c0[i] as f32) * t) as u8;
        *p = Rgba([m(0), m(1), m(2), 255]);
    }
    let mut d = Document::new(img);
    let (wf, hf) = (w as f32, h as f32);
    let (mx, my) = (wf * 0.08, hf * 0.1);
    // window body, title bar
    d.add(
        Shape::Rect {
            a: px(mx, my),
            b: px(wf - mx, hf - my),
            fill: true,
        },
        [246, 247, 250, 255],
        1.0,
    );
    d.add(
        Shape::Rect {
            a: px(mx, my),
            b: px(wf - mx, my + 34.0),
            fill: true,
        },
        [34, 36, 44, 255],
        1.0,
    );
    for (i, c) in [[237, 106, 94, 255], [245, 191, 79, 255], [97, 197, 84, 255]]
        .into_iter()
        .enumerate()
    {
        d.add(
            Shape::Ellipse {
                a: px(mx + 12.0 + i as f32 * 22.0, my + 11.0),
                b: px(mx + 24.0 + i as f32 * 22.0, my + 23.0),
                fill: true,
            },
            c,
            1.0,
        );
    }
    d.add(
        Shape::Text {
            at: px(mx + 90.0, my + 7.0),
            text: TITLES[seed % TITLES.len()].to_owned(),
            size: 17.0,
        },
        [225, 228, 236, 255],
        1.0,
    );
    // text lines
    let mut y = my + 60.0;
    let mut n = 0;
    while y < hf - my - 150.0 {
        let len = (wf - 2.0 * mx - 60.0) * (0.45 + 0.5 * (((n * 37 + seed * 11) % 10) as f32 / 10.0));
        d.add(
            Shape::Rect {
                a: px(mx + 24.0, y),
                b: px(mx + 24.0 + len, y + 10.0),
                fill: true,
            },
            [200, 205, 215, 255],
            1.0,
        );
        y += 24.0;
        n += 1;
    }
    // bar chart
    let base = hf - my - 24.0;
    for i in 0..7 {
        let bh = 30.0 + ((i * 53 + seed * 29) % 90) as f32;
        let x = mx + 30.0 + i as f32 * 46.0;
        d.add(
            Shape::Rect {
                a: px(x, base - bh),
                b: px(x + 30.0, base),
                fill: true,
            },
            [c1[0], c1[1], c1[2], 255],
            1.0,
        );
    }
    // a "sensitive" line that demos blur / redaction
    d.add(
        Shape::Text {
            at: px(wf - mx - 330.0, my + 56.0),
            text: "client: jane.doe@example.com".into(),
            size: 18.0,
        },
        [30, 33, 44, 255],
        1.0,
    );
    d.add(
        Shape::Text {
            at: px(wf - mx - 330.0, my + 86.0),
            text: "phone: +1 202 555 0143".into(),
            size: 18.0,
        },
        [30, 33, 44, 255],
        1.0,
    );
    d.render()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The review screenshot depends on real OCR finding the invented e-mail address and phone number.
    #[test]
    fn real_ocr_sees_the_invented_contact_details() {
        if snapforge_core::ocr::available_languages().is_empty() {
            return;
        }
        let data = snapforge_core::ocr::recognize(&demo_image(2, 1100, 640), None).expect("ocr");
        eprintln!("OCR of the demo picture: {:?}", data.text);
        let found = snapforge_core::sensitive::find(&data.lines, 3);
        assert!(
            found.len() >= 2,
            "e-mail and phone are found, got {found:?} in {:?}",
            data.text
        );
    }

    #[test]
    fn demo_images_are_deterministic_and_varied() {
        let a = demo_image(0, 640, 400);
        assert_eq!(a.dimensions(), (640, 400));
        assert_eq!(a, demo_image(0, 640, 400));
        assert_ne!(a, demo_image(1, 640, 400));
        let distinct: std::collections::HashSet<_> = a.pixels().step_by(97).map(|p| p.0).collect();
        assert!(distinct.len() > 8, "not a flat colour");
    }
}
