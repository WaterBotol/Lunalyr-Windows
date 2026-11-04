//! Hangul to Revised Romanization of Korean.
//!
//! Syllables are decomposed into initial / vowel / final jamo. Within a
//! run of syllables the sound changes RR writes are applied: liaison of a
//! final consonant into a following ㅇ (읽어 ilgeo, 없어 eopseo, 좋아 joa),
//! palatalisation (같이 gachi), nasalisation (합니다 hamnida, 독립 dongnip),
//! lateralisation (신라 silla, 설날 seollal), ㄹ → n after other nasals
//! (종로 jongno) and aspiration with ㅎ (좋고 joko, 축하 chuka).
//! Tensification is not written, as in RR.

const INITIALS: [&str; 19] =
    ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
const VOWELS: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu",
    "ui", "i",
];

// Initial indices.
const G: usize = 0;
const N: usize = 2;
const D: usize = 3;
const R: usize = 5;
const M: usize = 6;
const B: usize = 7;
const S: usize = 9;
const SS: usize = 10;
const SILENT: usize = 11;
const J: usize = 12;
const CH: usize = 14;
const K: usize = 15;
const T: usize = 16;
const P: usize = 17;
const H: usize = 18;

/// Final index → (consonant left in place, consonant moved into a following
/// ㅇ as an initial index). Finals: 0 none, 1 ㄱ, 2 ㄲ, 3 ㄳ, 4 ㄴ, 5 ㄵ, 6 ㄶ,
/// 7 ㄷ, 8 ㄹ, 9 ㄺ, 10 ㄻ, 11 ㄼ, 12 ㄽ, 13 ㄾ, 14 ㄿ, 15 ㅀ, 16 ㅁ, 17 ㅂ,
/// 18 ㅄ, 19 ㅅ, 20 ㅆ, 21 ㅇ, 22 ㅈ, 23 ㅊ, 24 ㅋ, 25 ㅌ, 26 ㅍ, 27 ㅎ.
fn liaison(final_index: usize) -> (Coda, Option<usize>) {
    match final_index {
        1 => (Coda::None, Some(G)),
        2 => (Coda::None, Some(1)),
        3 => (Coda::K, Some(S)),
        4 => (Coda::None, Some(N)),
        5 => (Coda::N, Some(J)),
        6 => (Coda::None, Some(N)),
        7 => (Coda::None, Some(D)),
        8 => (Coda::None, Some(R)),
        9 => (Coda::L, Some(G)),
        10 => (Coda::L, Some(M)),
        11 => (Coda::L, Some(B)),
        12 => (Coda::L, Some(S)),
        13 => (Coda::L, Some(T)),
        14 => (Coda::L, Some(P)),
        15 => (Coda::None, Some(R)),
        16 => (Coda::None, Some(M)),
        17 => (Coda::None, Some(B)),
        18 => (Coda::P, Some(S)),
        19 => (Coda::None, Some(S)),
        20 => (Coda::None, Some(SS)),
        22 => (Coda::None, Some(J)),
        23 => (Coda::None, Some(CH)),
        24 => (Coda::None, Some(K)),
        25 => (Coda::None, Some(T)),
        26 => (Coda::None, Some(P)),
        _ => (Coda::None, None), // ㅎ is silent before a vowel
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Coda {
    None,
    K,
    N,
    T,
    L,
    M,
    P,
    Ng,
}

impl Coda {
    fn latin(self) -> &'static str {
        match self {
            Coda::None => "",
            Coda::K => "k",
            Coda::N => "n",
            Coda::T => "t",
            Coda::L => "l",
            Coda::M => "m",
            Coda::P => "p",
            Coda::Ng => "ng",
        }
    }
}

/// The neutralised coda of a final, and whether it carries ㅎ.
fn neutral(final_index: usize) -> (Coda, bool) {
    match final_index {
        0 => (Coda::None, false),
        1 | 2 | 3 | 9 | 24 => (Coda::K, false),
        4 | 5 => (Coda::N, false),
        6 => (Coda::N, true),
        7 | 19 | 20 | 22 | 23 | 25 => (Coda::T, false),
        8 | 11 | 12 | 13 => (Coda::L, false),
        15 => (Coda::L, true),
        10 | 16 => (Coda::M, false),
        14 | 17 | 18 | 26 => (Coda::P, false),
        21 => (Coda::Ng, false),
        _ => (Coda::T, true), // 27 ㅎ
    }
}

fn aspirate(initial: usize) -> usize {
    match initial {
        G => K,
        D => T,
        B => P,
        J => CH,
        other => other,
    }
}

struct Syllable {
    initial: usize,
    vowel: usize,
    last: usize,
}

fn decompose(c: char) -> Option<Syllable> {
    let code = c as u32;
    if !(0xAC00..=0xD7A3).contains(&code) {
        return None;
    }
    let index = (code - 0xAC00) as usize;
    Some(Syllable { initial: index / (21 * 28), vowel: (index % (21 * 28)) / 28, last: index % 28 })
}

/// Romanises one run of consecutive Hangul syllables.
fn run(syllables: &[Syllable]) -> String {
    let mut initials: Vec<usize> = syllables.iter().map(|s| s.initial).collect();
    // ㄹ written "l" after a final ㄹ.
    let mut lateral = vec![false; syllables.len()];
    let mut out = String::new();
    for (i, syllable) in syllables.iter().enumerate() {
        out.push_str(if lateral[i] && initials[i] == R { "l" } else { INITIALS[initials[i]] });
        out.push_str(VOWELS[syllable.vowel]);
        let Some(next) = syllables.get(i + 1) else {
            out.push_str(neutral(syllable.last).0.latin());
            break;
        };
        let next_initial = initials[i + 1];
        if syllable.last == 0 {
            continue;
        }
        if next_initial == SILENT {
            let (stay, moved) = liaison(syllable.last);
            out.push_str(stay.latin());
            if let Some(mut moved) = moved {
                // Palatalisation: ㄷ/ㅌ + 이 → ji / chi.
                if next.vowel == 20 && (moved == D || moved == T) {
                    moved = if moved == D { J } else { CH };
                }
                initials[i + 1] = moved;
            }
            continue;
        }
        let (mut coda, has_h) = neutral(syllable.last);
        let mut follow = next_initial;
        if has_h {
            match follow {
                G | D | J => {
                    follow = aspirate(follow);
                    coda = if coda == Coda::T { Coda::None } else { coda };
                }
                S => {
                    follow = SS;
                    coda = if coda == Coda::T { Coda::None } else { coda };
                }
                N => coda = if coda == Coda::T { Coda::N } else { coda },
                _ => {}
            }
        } else if follow == H && matches!(coda, Coda::K | Coda::T | Coda::P) {
            follow = match coda {
                Coda::K => K,
                Coda::T => {
                    if syllable.last == 22 {
                        CH
                    } else {
                        T
                    }
                }
                _ => P,
            };
            coda = Coda::None;
        }
        match (coda, follow) {
            (Coda::K, N | M) => coda = Coda::Ng,
            (Coda::T, N | M) => coda = Coda::N,
            (Coda::P, N | M) => coda = Coda::M,
            (Coda::K, R) => {
                coda = Coda::Ng;
                follow = N;
            }
            (Coda::P, R) => {
                coda = Coda::M;
                follow = N;
            }
            (Coda::T, R) => {
                coda = Coda::N;
                follow = N;
            }
            (Coda::M | Coda::Ng, R) => follow = N,
            (Coda::N, R) => {
                coda = Coda::L;
                lateral[i + 1] = true;
            }
            (Coda::L, N) => {
                follow = R;
                lateral[i + 1] = true;
            }
            (Coda::L, R) => lateral[i + 1] = true,
            _ => {}
        }
        out.push_str(coda.latin());
        initials[i + 1] = follow;
    }
    out
}

/// Revised Romanization; non-Hangul characters pass through and separate
/// runs (no sound changes across them).
pub fn hangul_to_latin(text: &str) -> String {
    let mut out = String::new();
    let mut pending: Vec<Syllable> = Vec::new();
    for c in text.chars() {
        if let Some(syllable) = decompose(c) {
            pending.push(syllable);
        } else {
            if !pending.is_empty() {
                out.push_str(&run(&pending));
                pending.clear();
            }
            out.push(c);
        }
    }
    if !pending.is_empty() {
        out.push_str(&run(&pending));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_syllables() {
        assert_eq!(hangul_to_latin("안녕하세요"), "annyeonghaseyo");
        assert_eq!(hangul_to_latin("사랑해"), "saranghae");
        assert_eq!(hangul_to_latin("서울"), "seoul");
        assert_eq!(hangul_to_latin("라면"), "ramyeon");
        assert_eq!(hangul_to_latin("의사"), "uisa");
        assert_eq!(hangul_to_latin("부엌"), "bueok");
    }

    #[test]
    fn liaison_and_palatalisation() {
        assert_eq!(hangul_to_latin("읽어"), "ilgeo");
        assert_eq!(hangul_to_latin("없어"), "eopseo");
        assert_eq!(hangul_to_latin("좋아"), "joa");
        assert_eq!(hangul_to_latin("싫어"), "sireo");
        assert_eq!(hangul_to_latin("같이"), "gachi");
        assert_eq!(hangul_to_latin("굳이"), "guji");
        assert_eq!(hangul_to_latin("음악"), "eumak");
        assert_eq!(hangul_to_latin("한국어"), "hangugeo");
    }

    #[test]
    fn assimilation() {
        assert_eq!(hangul_to_latin("감사합니다"), "gamsahamnida");
        assert_eq!(hangul_to_latin("독립"), "dongnip");
        assert_eq!(hangul_to_latin("종로"), "jongno");
        assert_eq!(hangul_to_latin("신라"), "silla");
        assert_eq!(hangul_to_latin("설날"), "seollal");
        assert_eq!(hangul_to_latin("좋고"), "joko");
        assert_eq!(hangul_to_latin("축하"), "chuka");
        assert_eq!(hangul_to_latin("놓는"), "nonneun");
        assert_eq!(hangul_to_latin("밥물"), "bammul");
        assert_eq!(hangul_to_latin("빨리"), "ppalli");
    }

    #[test]
    fn non_hangul_separates_runs() {
        assert_eq!(hangul_to_latin("밥 먹어!"), "bap meogeo!");
        assert_eq!(hangul_to_latin("I love 너"), "I love neo");
        assert_eq!(hangul_to_latin(""), "");
    }
}
