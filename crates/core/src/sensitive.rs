//! Finding sensitive text (e-mail addresses, phone numbers, keys and tokens) in OCR output, so the post-capture
//! chain can hide it. Pure logic: it works on recognised words and their rectangles.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

#[derive(Clone, Debug, PartialEq)]
pub struct OcrWord {
    pub text: String,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
}

impl OcrLine {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OcrData {
    pub text: String,
    pub lines: Vec<OcrLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Email,
    Phone,
    Secret,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub kind: Kind,
    pub rect: Rect,
    /// The matched text (kept in memory for the review screen only; never logged).
    pub text: String,
}

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,}").expect("regex")
});
static PHONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\+?\d[\d\s().\-]{7,}\d").expect("regex"));
static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        r"\bgh[pousr]_[A-Za-z0-9]{30,}\b",
        r"\bsk-[A-Za-z0-9_\-]{20,}",
        r"\bxox[baprs]-[A-Za-z0-9\-]{10,}",
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
        r"(?i)\bbot\d{6,}:[A-Za-z0-9_\-]{30,}",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        r"(?i)(?:api[_\-]?key|secret|token|passw(?:or)?d|pwd)\s*[:=]\s*\S{6,}",
    ]
    .iter()
    .map(|r| Regex::new(r).expect("regex"))
    .collect()
});
static LONG_TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9_\-+/=]{32,}\b").expect("regex"));

fn digits(s: &str) -> usize {
    s.chars().filter(char::is_ascii_digit).count()
}

/// A long random-looking run: needs digits and letters so ordinary long words and numbers do not match.
fn looks_random(s: &str) -> bool {
    let (mut d, mut l) = (0, 0);
    for c in s.chars() {
        if c.is_ascii_digit() {
            d += 1;
        } else if c.is_ascii_alphabetic() {
            l += 1;
        }
    }
    d >= 4 && l >= 4
}

/// All candidate spans, most specific kind first: when rules overlap, the first one wins.
fn spans(line: &str) -> Vec<(Kind, usize, usize)> {
    let mut out = Vec::new();
    for re in SECRETS.iter() {
        for m in re.find_iter(line) {
            out.push((Kind::Secret, m.start(), m.end()));
        }
    }
    for m in LONG_TOKEN.find_iter(line) {
        if looks_random(m.as_str()) {
            out.push((Kind::Secret, m.start(), m.end()));
        }
    }
    for m in EMAIL.find_iter(line) {
        out.push((Kind::Email, m.start(), m.end()));
    }
    for m in PHONE.find_iter(line) {
        if (10..=15).contains(&digits(m.as_str())) {
            out.push((Kind::Phone, m.start(), m.end()));
        }
    }
    out
}

/// Finds sensitive text; each match becomes one rectangle over the words it covers (inflated by `pad` pixels).
pub fn find(lines: &[OcrLine], pad: i32) -> Vec<Match> {
    let mut out: Vec<Match> = Vec::new();
    for line in lines {
        // character spans of every word inside the joined line text
        let mut at = 0usize;
        let mut bounds = Vec::with_capacity(line.words.len());
        for w in &line.words {
            bounds.push((at, at + w.text.len()));
            at += w.text.len() + 1;
        }
        let text = line.text();
        for (kind, s, e) in spans(&text) {
            let hit: Vec<&OcrWord> = line
                .words
                .iter()
                .zip(&bounds)
                .filter(|(_, (ws, we))| *ws < e && *we > s)
                .map(|(w, _)| w)
                .collect();
            let Some(first) = hit.first() else { continue };
            let rect = hit.iter().fold(first.rect, |acc, w| acc.union(&w.rect));
            let rect = Rect::new(rect.x - pad, rect.y - pad, rect.w + 2 * pad, rect.h + 2 * pad);
            // one hit per area: overlapping matches of different rules collapse into the first
            if !out
                .iter()
                .any(|m| m.rect.intersect(&rect).is_some_and(|i| i.area() * 2 > rect.area()))
            {
                out.push(Match {
                    kind,
                    rect,
                    text: text.get(s..e).unwrap_or("").to_owned(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One word per whitespace-separated token, laid out left to right, 10 px per character.
    fn line(text: &str, y: i32) -> OcrLine {
        let mut x = 0;
        let words = text
            .split(' ')
            .map(|t| {
                let w = OcrWord {
                    text: t.to_owned(),
                    rect: Rect::new(x, y, t.len() as i32 * 10, 20),
                };
                x += (t.len() as i32 + 1) * 10;
                w
            })
            .collect();
        OcrLine { words }
    }

    fn kinds(text: &str) -> Vec<Kind> {
        find(&[line(text, 0)], 0).into_iter().map(|m| m.kind).collect()
    }

    #[test]
    fn emails_are_found_with_a_rect_over_the_word() {
        let m = find(&[line("write to jane.doe@example.com today", 40)], 2);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].kind, Kind::Email);
        // "write to " is 9 characters = 90 px; the address is 20 characters wide
        assert_eq!(m[0].rect, Rect::new(88, 38, 204, 24));
        assert_eq!(m[0].text, "jane.doe@example.com");
    }

    #[test]
    fn phones_need_enough_digits() {
        assert_eq!(kinds("call +1 202 555 0143 now"), vec![Kind::Phone]);
        assert_eq!(kinds("tel: (495) 123-45-67"), vec![Kind::Phone]);
        assert!(
            kinds("order 12345 of 2026-10-15").is_empty(),
            "short numbers and dates are not phones"
        );
        assert!(kinds("version 1.2.3.4").is_empty());
    }

    #[test]
    fn well_known_key_shapes_are_found() {
        assert_eq!(kinds("id AKIAABCDEFGHIJKLMNOP end"), vec![Kind::Secret]);
        assert_eq!(kinds("ghp_abcdefghijklmnopqrstuvwxyz0123456789"), vec![Kind::Secret]);
        assert_eq!(kinds("sk-abcdefghijklmnopqrstuvwxyz"), vec![Kind::Secret]);
        assert_eq!(
            kinds("Authorization eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N"),
            vec![Kind::Secret]
        );
        assert_eq!(kinds("password: hunter2hunter2"), vec![Kind::Secret]);
    }

    #[test]
    fn long_random_strings_match_but_plain_words_do_not() {
        assert_eq!(
            kinds("key 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"),
            vec![Kind::Secret]
        );
        assert!(
            kinds("internationalizationinternationalizationxx").is_empty(),
            "long word without digits"
        );
        assert!(kinds("12345678901234567890123456789012345").is_empty(), "digits only");
    }

    #[test]
    fn ordinary_text_has_no_findings() {
        assert!(kinds("Quarterly report for the sales team").is_empty());
        assert!(kinds("see https://example.com/docs for details").is_empty());
    }

    #[test]
    fn several_findings_in_one_line_and_across_lines() {
        let m = find(&[line("a@b.co and c@d.org", 0), line("call 555 123 4567", 30)], 0);
        assert_eq!(m.iter().filter(|m| m.kind == Kind::Email).count(), 2);
        assert_eq!(m.iter().filter(|m| m.kind == Kind::Phone).count(), 1);
    }

    #[test]
    fn overlapping_rules_give_one_rectangle() {
        // "token=<email-looking>" matches both the email rule and the key=value rule
        let m = find(&[line("token=abcdef@example.com", 0)], 0);
        assert_eq!(m.len(), 1);
    }
}
