//! Platform-independent core of Lunalyr for Windows.
//!
//! Ported from the macOS app (Swift) at github.com/WaterBotol/Lunalyr, which
//! is itself a modification of Verso (GPLv3 + Commons Clause). Nothing in this
//! crate touches Windows APIs, so it builds and tests on any host.
//!
//! * [`model`] — lyric lines/words, queries, candidates (shared with the renderer).
//! * [`contract`] — the JSON shapes sent to the TypeScript renderer.
//! * [`settings`] — persisted user settings with defaults and clamping.
//! * [`timing`] — [`timing::make_line`], [`timing::distribute_words`],
//!   [`timing::valid_word_timing`], [`timing::has_word_timing`],
//!   [`timing::timing_tier`], [`timing::precision`], [`timing::LineIndex`]
//!   (active line: last start at or before `t`), [`timing::vocal_windows`] +
//!   [`timing::vocal_gap`] (instrumental interludes),
//!   [`timing::estimated_line_end`], [`timing::repair_qq`] and
//!   [`timing::prepare`] (run on every lookup result before display).
//! * [`parse`] — [`parse::parse_lrc`] (offsets, multi-stamp rows, enhanced
//!   word stamps), [`parse::parse_yrc`] (NetEase), [`parse::parse_lyricsfile`]
//!   (LRCLIB YAML), [`parse::parse_ttml`], [`parse::parse_import`] (user
//!   import: TTML or LRC) and [`parse::attach_readings`] (`.roma.lrc`).
//! * [`rank`] — [`rank::match_score`], [`rank::ranked`], [`rank::choose`] /
//!   [`rank::choose_index`].
//! * [`http`] — the [`http::HttpClient`] trait, [`http::ReqwestClient`]
//!   (OS TLS) and [`http::FixtureHttp`] for tests.
//! * [`providers`] — the [`providers::Provider`] trait,
//!   [`providers::provider_for`] and every source: [`providers::LocalFiles`],
//!   [`providers::LyricsPlus`], [`providers::NetEase`], [`providers::Lrclib`],
//!   [`providers::QqMusic`], [`providers::Kugou`], [`providers::Kuwo`],
//!   [`providers::LyricsOvh`]. [`providers::Unavailable`] reports
//!   "Not available in this version" for a source without an implementation.
//! * [`cache`] — [`cache::LyricCache`]: automatic (14 days, 150 files) and
//!   pinned lyric choices on disk.
//! * [`lookup`] — [`lookup::LyricsService`] (`lookup`, `pin`, `unpin`),
//!   [`lookup::LookupReport`] (`info()` → `contract::LookupInfo`),
//!   [`lookup::summary`] / [`lookup::summaries`] for
//!   `contract::CandidateSummary`.
//! * [`palette`] — [`palette::extract_palette`] (album colours, 3×3 HSB) and
//!   [`palette::fallback_palette`].
//! * [`romanize`] — [`romanize::readings`] (per-line reading words: pinyin,
//!   Korean Revised Romanization, kana → Hepburn, Japanese kanji readings
//!   from a platform callback such as `JapanesePhoneticAnalyzer`).

pub mod cache;
pub mod contract;
pub mod http;
pub mod lookup;
pub mod model;
pub mod palette;
pub mod parse;
pub mod providers;
pub mod rank;
pub mod romanize;
pub mod settings;
mod text;
pub mod timing;
