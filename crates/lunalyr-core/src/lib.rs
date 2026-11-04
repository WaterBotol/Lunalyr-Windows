//! Platform-independent core of Lunalyr for Windows.
//!
//! Ported from the macOS app (Swift) at github.com/WaterBotol/Lunalyr, which
//! is itself a modification of Verso (GPLv3 + Commons Clause). Nothing in this
//! crate touches Windows APIs, so it builds and tests on any host.
//!
//! * [`model`] — lyric lines/words, queries, candidates (shared with the renderer).
//! * [`contract`] — the JSON shapes sent to the TypeScript renderer.
//! * [`settings`] — persisted user settings with defaults and clamping.

pub mod contract;
pub mod model;
pub mod settings;
