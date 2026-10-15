//! Charts as editable objects: bar, line and pie charts drawn from a small table (typed in, or pasted as CSV/TSV).

use serde::{Deserialize, Serialize};
use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Transform};

use super::model::{Color, Pt, norm_rect};
use super::render::{draw_text, measure_text, paint, stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub name: String,
    pub color: Color,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartSpec {
    pub kind: ChartKind,
    pub title: String,
    pub labels: Vec<String>,
    pub series: Vec<Series>,
    pub show_legend: bool,
    pub show_values: bool,
}

pub const PALETTE: [Color; 8] = [
    [66, 133, 244, 255],
    [234, 67, 53, 255],
    [251, 188, 5, 255],
    [52, 168, 83, 255],
    [171, 71, 188, 255],
    [255, 112, 67, 255],
    [0, 172, 193, 255],
    [124, 179, 66, 255],
];

impl ChartSpec {
    pub fn sample(kind: ChartKind) -> ChartSpec {
        ChartSpec {
            kind,
            title: "Sales".into(),
            labels: ["Q1", "Q2", "Q3", "Q4"].iter().map(|s| (*s).to_owned()).collect(),
            series: vec![Series {
                name: "2026".into(),
                color: PALETTE[0],
                values: vec![12.0, 19.0, 8.0, 15.0],
            }],
            show_legend: true,
            show_values: true,
        }
    }

    /// Adds or removes categories so every series has exactly `labels.len()` values.
    pub fn normalize(&mut self) {
        let n = self.labels.len();
        for s in &mut self.series {
            s.values.resize(n, 0.0);
        }
    }
}

// ----------------------------------------------------------------------------------------------- table parsing

/// Splits pasted text into rows of cells. The delimiter (tab, `;` or `,`) is detected from the data; double quotes
/// protect delimiters inside a cell.
pub fn parse_table(text: &str) -> Vec<Vec<String>> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let count = |l: &str, d: char| {
        let mut inq = false;
        l.chars()
            .filter(|&c| {
                if c == '"' {
                    inq = !inq;
                }
                c == d && !inq
            })
            .count()
    };
    let delim = ['\t', ';', ',']
        .into_iter()
        .max_by_key(|&d| lines.iter().map(|l| count(l, d)).min().unwrap_or(0))
        .filter(|&d| lines.iter().all(|l| count(l, d) > 0))
        .unwrap_or(',');
    lines.iter().map(|l| split_line(l, delim)).collect()
}

fn split_line(line: &str, delim: char) -> Vec<String> {
    let (mut cells, mut cur, mut inq) = (Vec::new(), String::new(), false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if inq && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => inq = !inq,
            c if c == delim && !inq => cells.push(std::mem::take(&mut cur).trim().to_owned()),
            c => cur.push(c),
        }
    }
    cells.push(cur.trim().to_owned());
    cells
}

/// `1 234,5` / `1,234.5` / `12%` → number. Decimal comma is accepted unless the cell also has a dot.
pub fn parse_number(cell: &str) -> Option<f64> {
    let t: String = cell
        .trim()
        .trim_end_matches('%')
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
        .collect();
    if t.is_empty() {
        return None;
    }
    let normalized = match (t.contains(','), t.contains('.')) {
        (true, true) => t.replace(',', ""),
        (true, false) => {
            // "1,234" with exactly three digits after the comma is a thousands separator only if there are several groups
            if t.matches(',').count() > 1 {
                t.replace(',', "")
            } else {
                t.replace(',', ".")
            }
        }
        _ => t,
    };
    normalized.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Builds a chart from table rows: an optional header row (names of the series), first column = category labels.
pub fn spec_from_table(rows: &[Vec<String>], kind: ChartKind) -> Result<ChartSpec, String> {
    if rows.is_empty() {
        return Err("the table is empty".into());
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width < 2 {
        return Err("need at least two columns: labels and values".into());
    }
    // the first row is a header when its value cells are not numbers
    let header = rows[0].iter().skip(1).all(|c| parse_number(c).is_none());
    let (head, body) = if header {
        (Some(&rows[0]), &rows[1..])
    } else {
        (None, rows)
    };
    if body.is_empty() {
        return Err("there are no data rows".into());
    }
    let labels: Vec<String> = body.iter().map(|r| r.first().cloned().unwrap_or_default()).collect();
    let series = (1..width)
        .map(|c| Series {
            name: head
                .and_then(|h| h.get(c).cloned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Series {c}")),
            color: PALETTE[(c - 1) % PALETTE.len()],
            values: body
                .iter()
                .map(|r| r.get(c).and_then(|x| parse_number(x)).unwrap_or(0.0))
                .collect(),
        })
        .collect();
    Ok(ChartSpec {
        kind,
        title: String::new(),
        labels,
        series,
        show_legend: width > 2 || kind == ChartKind::Pie,
        show_values: true,
    })
}

// ----------------------------------------------------------------------------------------------------- drawing

/// "Nice" axis ticks covering `[lo, hi]`: returns `(first, step, count)`.
pub fn nice_ticks(lo: f64, hi: f64, target: usize) -> (f64, f64, usize) {
    let span = (hi - lo).max(1e-9);
    let raw = span / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    let first = (lo / step).floor() * step;
    let count = ((hi - first) / step).ceil() as usize + 1;
    (first, step, count.max(2))
}

fn fmt_value(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn rect_path(x: f32, y: f32, w: f32, h: f32) -> Option<tiny_skia::Path> {
    tiny_skia::Rect::from_xywh(x, y, w.max(0.5), h.max(0.5)).map(PathBuilder::from_rect)
}

/// Draws the chart into the box spanned by `a`/`b` (image pixels).
pub fn draw(pm: &mut Pixmap, a: Pt, b: Pt, spec: &ChartSpec, ink: Color) {
    let (lo, hi) = norm_rect(a, b);
    let (x, y, w, h) = (lo.x, lo.y, hi.x - lo.x, hi.y - lo.y);
    if w < 40.0 || h < 40.0 {
        return;
    }
    let id = Transform::identity();
    if let Some(p) = rect_path(x, y, w, h) {
        pm.fill_path(&p, &paint([255, 255, 255, 240]), FillRule::Winding, id, None);
        pm.stroke_path(&p, &paint([170, 175, 185, 255]), &stroke(1.0, LineCap::Butt), id, None);
    }
    let text_ink = if ink[3] == 0 { [30, 33, 44, 255] } else { ink };
    let fs = (h / 22.0).clamp(11.0, 18.0);
    let mut top = y + 8.0;
    if !spec.title.is_empty() {
        let (tw, th) = measure_text(&spec.title, fs * 1.25);
        draw_text(pm, &spec.title, Pt::new(x + (w - tw) / 2.0, top), fs * 1.25, text_ink);
        top += th + 6.0;
    }
    let mut bottom = y + h - 8.0;
    if spec.show_legend && (spec.series.len() > 1 || spec.kind == ChartKind::Pie) {
        let names: Vec<(String, Color)> = if spec.kind == ChartKind::Pie {
            spec.labels
                .iter()
                .enumerate()
                .map(|(i, l)| (l.clone(), PALETTE[i % PALETTE.len()]))
                .collect()
        } else {
            spec.series.iter().map(|s| (s.name.clone(), s.color)).collect()
        };
        let mut cx = x + 12.0;
        let mut cy = bottom - fs - 2.0;
        for (name, color) in names {
            let (tw, _) = measure_text(&name, fs);
            if cx + tw + fs + 18.0 > x + w - 8.0 && cx > x + 14.0 {
                cx = x + 12.0;
                cy -= fs + 6.0;
            }
            if let Some(p) = rect_path(cx, cy + 2.0, fs - 2.0, fs - 2.0) {
                pm.fill_path(&p, &paint(color), FillRule::Winding, id, None);
            }
            draw_text(pm, &name, Pt::new(cx + fs + 2.0, cy), fs, text_ink);
            cx += tw + fs + 18.0;
        }
        bottom = cy - 6.0;
    }
    match spec.kind {
        ChartKind::Pie => draw_pie(pm, spec, (x, top, w, bottom - top), fs, text_ink),
        ChartKind::Bar | ChartKind::Line => draw_axes_chart(pm, spec, (x, top, w, bottom - top), fs, text_ink),
    }
}

fn draw_axes_chart(pm: &mut Pixmap, spec: &ChartSpec, area: (f32, f32, f32, f32), fs: f32, ink: Color) {
    let id = Transform::identity();
    let (ax, ay, aw, ah) = area;
    let all = spec.series.iter().flat_map(|s| s.values.iter().copied());
    let (mut vmin, mut vmax) = all.fold((0.0f64, f64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if vmax == f64::MIN {
        vmax = 1.0;
    }
    if vmax <= vmin {
        vmax = vmin + 1.0;
    }
    let (first, step, count) = nice_ticks(vmin, vmax, 5);
    vmin = vmin.min(first);
    let top_v = first + step * (count - 1) as f64;
    let label_w = (0..count)
        .map(|i| measure_text(&fmt_value(first + step * i as f64), fs).0)
        .fold(0.0, f32::max);
    let (px, py) = (ax + label_w + 14.0, ay + fs);
    let (pw, ph) = (aw - (label_w + 14.0) - 14.0, ah - fs - fs * 1.6);
    if pw < 20.0 || ph < 20.0 {
        return;
    }
    let ymap = |v: f64| py + ph - ((v - vmin) / (top_v - vmin).max(1e-9)) as f32 * ph;
    // grid + value labels
    for i in 0..count {
        let v = first + step * i as f64;
        let gy = ymap(v);
        let mut pb = PathBuilder::new();
        pb.move_to(px, gy);
        pb.line_to(px + pw, gy);
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint([215, 218, 225, 255]), &stroke(1.0, LineCap::Butt), id, None);
        }
        let t = fmt_value(v);
        let (tw, th) = measure_text(&t, fs);
        draw_text(pm, &t, Pt::new(px - tw - 6.0, gy - th / 2.0), fs, ink);
    }
    let n = spec.labels.len().max(1);
    let slot = pw / n as f32;
    let zero_y = ymap(0.0f64.max(vmin));
    for (i, label) in spec.labels.iter().enumerate() {
        let (tw, _) = measure_text(label, fs);
        let cx = px + slot * (i as f32 + 0.5);
        draw_text(pm, label, Pt::new(cx - tw.min(slot) / 2.0, py + ph + 4.0), fs, ink);
    }
    match spec.kind {
        ChartKind::Bar => {
            let sn = spec.series.len().max(1);
            let group = slot * 0.72;
            let bw = group / sn as f32;
            for (si, s) in spec.series.iter().enumerate() {
                for (i, v) in s.values.iter().enumerate().take(n) {
                    let bx = px + slot * i as f32 + (slot - group) / 2.0 + bw * si as f32;
                    let vy = ymap(*v);
                    let (y0, y1) = if vy < zero_y { (vy, zero_y) } else { (zero_y, vy) };
                    if let Some(p) = rect_path(bx + 1.0, y0, bw - 2.0, y1 - y0) {
                        pm.fill_path(&p, &paint(s.color), FillRule::Winding, id, None);
                    }
                    if spec.show_values {
                        let t = fmt_value(*v);
                        let (tw, th) = measure_text(&t, fs * 0.9);
                        draw_text(pm, &t, Pt::new(bx + (bw - tw) / 2.0, y0 - th - 1.0), fs * 0.9, ink);
                    }
                }
            }
        }
        _ => {
            for s in &spec.series {
                let pts: Vec<Pt> = s
                    .values
                    .iter()
                    .enumerate()
                    .take(n)
                    .map(|(i, v)| Pt::new(px + slot * (i as f32 + 0.5), ymap(*v)))
                    .collect();
                let mut pb = PathBuilder::new();
                for (i, p) in pts.iter().enumerate() {
                    if i == 0 {
                        pb.move_to(p.x, p.y)
                    } else {
                        pb.line_to(p.x, p.y)
                    }
                }
                if let Some(path) = pb.finish() {
                    pm.stroke_path(&path, &paint(s.color), &stroke(2.5, LineCap::Round), id, None);
                }
                for (p, v) in pts.iter().zip(&s.values) {
                    let mut dot = PathBuilder::new();
                    dot.push_circle(p.x, p.y, 3.5);
                    if let Some(d) = dot.finish() {
                        pm.fill_path(&d, &paint(s.color), FillRule::Winding, id, None);
                    }
                    if spec.show_values {
                        let t = fmt_value(*v);
                        let (tw, th) = measure_text(&t, fs * 0.9);
                        draw_text(pm, &t, Pt::new(p.x - tw / 2.0, p.y - th - 5.0), fs * 0.9, ink);
                    }
                }
            }
        }
    }
    // axes
    let mut pb = PathBuilder::new();
    pb.move_to(px, py);
    pb.line_to(px, py + ph);
    pb.line_to(px + pw, py + ph);
    if let Some(p) = pb.finish() {
        pm.stroke_path(&p, &paint([120, 125, 135, 255]), &stroke(1.2, LineCap::Butt), id, None);
    }
}

fn draw_pie(pm: &mut Pixmap, spec: &ChartSpec, area: (f32, f32, f32, f32), fs: f32, ink: Color) {
    let (ax, ay, aw, ah) = area;
    let Some(series) = spec.series.first() else { return };
    let values: Vec<f64> = series.values.iter().map(|v| v.max(0.0)).collect();
    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return;
    }
    let r = (aw.min(ah) / 2.0 - fs * 2.2).max(10.0);
    let (cx, cy) = (ax + aw / 2.0, ay + ah / 2.0);
    let mut angle = -std::f64::consts::FRAC_PI_2;
    for (i, v) in values.iter().enumerate() {
        let sweep = v / total * std::f64::consts::TAU;
        if sweep <= 0.0 {
            continue;
        }
        let steps = ((sweep / 0.05).ceil() as usize).max(2);
        let mut pb = PathBuilder::new();
        pb.move_to(cx, cy);
        for s in 0..=steps {
            let a = angle + sweep * s as f64 / steps as f64;
            pb.line_to(cx + r * a.cos() as f32, cy + r * a.sin() as f32);
        }
        pb.close();
        if let Some(p) = pb.finish() {
            let mut paint_ = Paint::default();
            let c = PALETTE[i % PALETTE.len()];
            paint_.set_color_rgba8(c[0], c[1], c[2], 255);
            paint_.anti_alias = true;
            pm.fill_path(&p, &paint_, FillRule::Winding, Transform::identity(), None);
            pm.stroke_path(
                &p,
                &paint([255, 255, 255, 255]),
                &stroke(1.5, LineCap::Butt),
                Transform::identity(),
                None,
            );
        }
        if spec.show_values {
            let mid = angle + sweep / 2.0;
            let t = format!("{:.0}%", v / total * 100.0);
            let (tw, th) = measure_text(&t, fs);
            let lr = r + fs * 0.9 + tw / 2.0;
            draw_text(
                pm,
                &t,
                Pt::new(
                    cx + lr * mid.cos() as f32 - tw / 2.0,
                    cy + lr * mid.sin() as f32 - th / 2.0,
                ),
                fs,
                ink,
            );
        }
        angle += sweep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Pt;

    fn rows(t: &str) -> Vec<Vec<String>> {
        parse_table(t)
    }

    #[test]
    fn tab_separated_paste_from_a_spreadsheet() {
        let r = rows("Month\tSales\tCosts\nJan\t12\t7\nFeb\t19,5\t8\n");
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], vec!["Month", "Sales", "Costs"]);
        assert_eq!(r[2], vec!["Feb", "19,5", "8"]);
    }

    #[test]
    fn csv_with_commas_semicolons_and_quotes() {
        assert_eq!(rows("a,b\nc,d")[1], vec!["c", "d"]);
        assert_eq!(rows("a;b\nc;d")[0], vec!["a", "b"]);
        assert_eq!(rows("name,val\n\"Smith, J\",5")[1], vec!["Smith, J", "5"]);
        assert_eq!(
            rows("x;y\n1,5;2,5")[1],
            vec!["1,5", "2,5"],
            "semicolon file keeps decimal commas"
        );
        assert!(rows("").is_empty());
    }

    #[test]
    fn numbers_in_several_notations() {
        assert_eq!(parse_number("12"), Some(12.0));
        assert_eq!(parse_number("19,5"), Some(19.5));
        assert_eq!(parse_number("1,234.5"), Some(1234.5));
        assert_eq!(parse_number("1 234"), Some(1234.0));
        assert_eq!(parse_number("12%"), Some(12.0));
        assert_eq!(parse_number("1,234,567"), Some(1234567.0));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number(""), None);
    }

    #[test]
    fn spec_from_a_table_with_and_without_header() {
        let s = spec_from_table(&rows("Month\tSales\tCosts\nJan\t12\t7\nFeb\t19\t8"), ChartKind::Bar).unwrap();
        assert_eq!(s.labels, vec!["Jan", "Feb"]);
        assert_eq!(s.series.len(), 2);
        assert_eq!(s.series[1].name, "Costs");
        assert_eq!(s.series[0].values, vec![12.0, 19.0]);
        assert!(s.show_legend, "two series need a legend");
        let s = spec_from_table(&rows("Jan,5\nFeb,7"), ChartKind::Pie).unwrap();
        assert_eq!(s.labels, vec!["Jan", "Feb"], "no header: the first row is data");
        assert_eq!(s.series[0].name, "Series 1");
        assert!(spec_from_table(&rows("only"), ChartKind::Bar).is_err());
        assert!(spec_from_table(&[], ChartKind::Bar).is_err());
        assert!(
            spec_from_table(&rows("a,b"), ChartKind::Bar)
                .unwrap_err()
                .contains("no data rows")
        );
    }

    #[test]
    fn nice_ticks_are_round_and_cover_the_range() {
        let (first, step, count) = nice_ticks(0.0, 19.0, 5);
        assert_eq!((first, step), (0.0, 5.0));
        assert!(first + step * (count - 1) as f64 >= 19.0);
        let (first, step, _) = nice_ticks(-3.0, 8.0, 5);
        assert!(first <= -3.0 && step > 0.0);
        let (_, step, count) = nice_ticks(0.0, 0.0, 5);
        assert!(step > 0.0 && count >= 2, "all-zero data still gives an axis");
    }

    #[test]
    fn normalize_resizes_series_to_the_labels() {
        let mut s = ChartSpec::sample(ChartKind::Bar);
        s.labels.push("Q5".into());
        s.normalize();
        assert_eq!(s.series[0].values.len(), 5);
        assert_eq!(s.series[0].values[4], 0.0);
        s.labels.truncate(2);
        s.normalize();
        assert_eq!(s.series[0].values.len(), 2);
    }

    #[test]
    fn spec_roundtrips_through_json() {
        let s = ChartSpec::sample(ChartKind::Line);
        let back: ChartSpec = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    fn render_chart(spec: ChartSpec) -> image::RgbaImage {
        use crate::editor::{Document, Shape};
        let mut d = Document::new(image::RgbaImage::from_pixel(420, 300, image::Rgba([60, 60, 70, 255])));
        d.add(
            Shape::Chart {
                a: Pt::new(10.0, 10.0),
                b: Pt::new(410.0, 290.0),
                spec,
            },
            [0, 0, 0, 0],
            1.0,
        );
        d.render()
    }

    fn count_colour(img: &image::RgbaImage, c: Color) -> usize {
        img.pixels().filter(|p| p.0[..3] == c[..3]).count()
    }

    #[test]
    fn every_chart_kind_draws_its_series_colour() {
        for kind in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
            let spec = ChartSpec::sample(kind);
            let img = render_chart(spec.clone());
            let c = if kind == ChartKind::Pie {
                PALETTE[0]
            } else {
                spec.series[0].color
            };
            assert!(count_colour(&img, c) > 200, "{kind:?} shows the series colour");
            assert_eq!(
                img.get_pixel(2, 2).0[..3],
                [60, 60, 70],
                "nothing outside the chart box"
            );
        }
    }

    #[test]
    fn chart_stays_editable_data_changes_the_picture() {
        let mut spec = ChartSpec::sample(ChartKind::Bar);
        let before = render_chart(spec.clone());
        spec.series[0].values[1] = 3.0;
        let after = render_chart(spec.clone());
        assert_ne!(before, after);
        spec.series[0].color = PALETTE[1];
        assert!(count_colour(&render_chart(spec), PALETTE[1]) > 200);
    }

    #[test]
    fn bigger_values_make_taller_bars() {
        let mut spec = ChartSpec::sample(ChartKind::Bar);
        spec.show_values = false;
        spec.series[0].values = vec![5.0, 20.0, 5.0, 5.0];
        let img = render_chart(spec.clone());
        let height = |x: u32| {
            (0..300u32)
                .filter(|&y| img.get_pixel(x, y).0[..3] == spec.series[0].color[..3])
                .count()
        };
        // bar 2 (centre x ≈ 172) is clearly taller than bar 1 (≈ 87)
        let tall = (150..200).map(height).max().unwrap();
        let short = (70..110).map(height).max().unwrap();
        assert!(tall > short * 2, "{tall} vs {short}");
    }

    #[test]
    fn tiny_or_empty_charts_do_not_panic() {
        let mut spec = ChartSpec::sample(ChartKind::Pie);
        spec.series[0].values = vec![0.0; 4];
        render_chart(spec);
        let mut e = ChartSpec::sample(ChartKind::Line);
        e.labels.clear();
        e.series.clear();
        render_chart(e);
    }
}
