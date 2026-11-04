//! Small string helpers that reproduce the Foundation behaviour the macOS
//! sources rely on (`CharacterSet.whitespaces`, `Character.isLetter`, String
//! equality under canonical equivalence, ...). Crate-internal.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// `CharacterSet.newlines`: U+000A–U+000D, U+0085, U+2028, U+2029.
pub(crate) fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `CharacterSet.whitespaces`: general category Zs plus TAB (no newlines).
pub(crate) fn is_space(c: char) -> bool {
    c == '\t' || (c.is_whitespace() && !is_newline(c))
}

/// `trimmingCharacters(in: .whitespaces)`.
pub(crate) fn trim_spaces(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// `trimmingCharacters(in: .whitespacesAndNewlines)`.
pub(crate) fn trim_all(s: &str) -> &str {
    s.trim_matches(char::is_whitespace)
}

/// `components(separatedBy: .newlines)`, except that CR LF counts as one
/// break (Foundation would yield an extra empty component).
pub(crate) fn split_newlines(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut iter = s.char_indices().peekable();
    while let Some((index, c)) = iter.next() {
        if is_newline(c) {
            parts.push(&s[start..index]);
            let mut next = index + c.len_utf8();
            if c == '\r' {
                if let Some(&(_, '\n')) = iter.peek() {
                    iter.next();
                    next += 1;
                }
            }
            start = next;
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Swift `Character.isLetter || Character.isNumber` for one grapheme
/// (both look at the first scalar).
pub(crate) fn grapheme_is_alnum(g: &str) -> bool {
    g.chars().next().is_some_and(char::is_alphanumeric)
}

/// Swift `Character.isWhitespace` for one grapheme.
pub(crate) fn grapheme_is_whitespace(g: &str) -> bool {
    g.chars().next().is_some_and(char::is_whitespace)
}

/// `text.filter { $0.isLetter || $0.isNumber }`.
pub(crate) fn letters_and_numbers(s: &str) -> String {
    s.graphemes(true).filter(|g| grapheme_is_alnum(g)).collect()
}

/// `text.filter { !$0.isWhitespace }`.
pub(crate) fn without_whitespace(s: &str) -> String {
    s.graphemes(true).filter(|g| !grapheme_is_whitespace(g)).collect()
}

/// Swift `String ==` compares canonical equivalents as equal.
pub(crate) fn canonical_eq(a: &str, b: &str) -> bool {
    a == b || a.nfc().eq(b.nfc())
}

/// Number of Swift `Character`s (extended grapheme clusters).
pub(crate) fn grapheme_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// `CharacterSet.alphanumerics` (categories L*, M*, N*).
pub(crate) fn is_cs_alphanumeric(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

/// Swift's `Sequence.min()` over doubles: the first element, replaced only
/// by strictly smaller ones.
pub(crate) fn swift_min(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut iter = values.into_iter();
    let mut result = iter.next()?;
    for value in iter {
        if value < result {
            result = value;
        }
    }
    Some(result)
}

/// Swift's `Sequence.max()` over doubles.
pub(crate) fn swift_max(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut iter = values.into_iter();
    let mut result = iter.next()?;
    for value in iter {
        if result < value {
            result = value;
        }
    }
    Some(result)
}

/// Lower-case hex SHA-256 of a UTF-8 string.
pub(crate) fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `LyricHTTP.text`: the handful of HTML entities lyric services emit.
pub(crate) fn decode_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_sets_match_foundation() {
        assert_eq!(trim_spaces("\t a b \u{3000}"), "a b");
        assert_eq!(trim_spaces("\na\n"), "\na\n");
        assert_eq!(trim_all("\n a \r\n"), "a");
    }

    #[test]
    fn newline_split_treats_crlf_as_one_break() {
        assert_eq!(split_newlines("a\r\nb\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(split_newlines(""), vec![""]);
        assert_eq!(split_newlines("a\n"), vec!["a", ""]);
    }

    #[test]
    fn letter_filter_keeps_whole_graphemes() {
        assert_eq!(letters_and_numbers("Sing, along! 42"), "Singalong42");
        assert_eq!(letters_and_numbers("e\u{301}!"), "e\u{301}");
        assert!(canonical_eq("\u{e9}", "e\u{301}"));
    }

    #[test]
    fn swift_min_max_keep_first_extreme() {
        assert_eq!(swift_min([3.0, 1.0, 2.0]), Some(1.0));
        assert_eq!(swift_max([3.0, 1.0, 5.0]), Some(5.0));
        assert_eq!(swift_max(std::iter::empty()), None);
    }
}
