//! Japanese readings from `Windows.Globalization.JapanesePhoneticAnalyzer`
//! (the Windows counterpart of the macOS tokenizer + transliteration).

use ::windows::core::HSTRING;
use ::windows::Globalization::JapanesePhoneticAnalyzer;

use super::util::ComApartment;
use crate::platform::logic;

/// `GetWords` accepts at most 100 characters per call.
const MAX_INPUT_UTF16: usize = 100;

/// Word-segmented readings of `text`, in order: `(DisplayText, YomiText)`
/// pairs, where YomiText is the hiragana reading of the word (kana and
/// non-Japanese words come back as they are). Longer text is analysed in
/// ≤ 100-character pieces cut at punctuation/whitespace. `None` if the
/// analyser is unavailable or fails. Callable from any thread.
pub fn japanese_readings(text: &str) -> Option<Vec<(String, String)>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    let _com = match ComApartment::multithreaded() {
        Ok(com) => com,
        Err(error) => {
            log::debug!("japanese: COM initialisation failed: {error}");
            return None;
        }
    };
    let mut readings = Vec::new();
    for chunk in logic::phonetic_chunks(text, MAX_INPUT_UTF16) {
        let words = match JapanesePhoneticAnalyzer::GetWords(&HSTRING::from(chunk)) {
            Ok(words) => words,
            Err(error) => {
                log::debug!("japanese: GetWords failed: {error}");
                return None;
            }
        };
        for word in &words {
            let display = word.DisplayText().ok()?.to_string_lossy();
            let reading = word.YomiText().ok()?.to_string_lossy();
            readings.push((display, reading));
        }
    }
    Some(readings)
}
