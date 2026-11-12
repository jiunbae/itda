//! Hangul typed from another computer, composed here.
//!
//! A viewer whose own input source is Korean sends what each key makes there: jamo (ㅎ, ㅏ, ㄴ),
//! one keysym at a time, because the screen it types into is not a text field its input method
//! composes in. Typed as they are, they stay apart ("ㅎㅏㄴ"). The sharing servers compose them
//! the way the standard 2-set (두벌식) keyboard does and keep replacing the syllable being typed:
//! ㅎ → 하 → 한, then ㄱ → 한ㄱ, ㅡ → 한그, ㄹ → 한글. This does not depend on the input source
//! of the computer being typed on. With ⌘/Ctrl/Alt held a jamo is a shortcut on its key
//! (⌘ㅊ is ⌘C): `latin` gives the key.

/// The compatibility jamo a keysym stands for: Unicode keysyms (noVNC: 0x0100_0000 | U+3131…)
/// or X11 Hangul keysyms (0x0ea1 Kiyeog … 0x0ed3 I, the same order as U+3131 … U+3163).
pub fn jamo(sym: u32) -> Option<char> {
    let cp = match sym {
        0x0100_3131..=0x0100_3163 => sym - 0x0100_0000,
        0x0ea1..=0x0ed3 => 0x3131 + (sym - 0x0ea1),
        _ => return None,
    };
    char::from_u32(cp)
}

/// The key a jamo is on in the 2-set layout (lower case; ㄲ ㄸ ㅃ ㅆ ㅉ ㅒ ㅖ are those keys
/// with Shift, which the viewer sends as its own key). None for jamo no single key types.
pub fn latin(j: char) -> Option<char> {
    const KEYS: [(char, char); 33] = [
        ('ㅂ', 'q'), ('ㅈ', 'w'), ('ㄷ', 'e'), ('ㄱ', 'r'), ('ㅅ', 't'), ('ㅛ', 'y'), ('ㅕ', 'u'), ('ㅑ', 'i'), ('ㅐ', 'o'), ('ㅔ', 'p'),
        ('ㅁ', 'a'), ('ㄴ', 's'), ('ㅇ', 'd'), ('ㄹ', 'f'), ('ㅎ', 'g'), ('ㅗ', 'h'), ('ㅓ', 'j'), ('ㅏ', 'k'), ('ㅣ', 'l'),
        ('ㅋ', 'z'), ('ㅌ', 'x'), ('ㅊ', 'c'), ('ㅍ', 'v'), ('ㅠ', 'b'), ('ㅜ', 'n'), ('ㅡ', 'm'),
        ('ㅃ', 'q'), ('ㅉ', 'w'), ('ㄸ', 'e'), ('ㄲ', 'r'), ('ㅆ', 't'), ('ㅒ', 'o'), ('ㅖ', 'p'),
    ];
    KEYS.iter().find(|(k, _)| *k == j).map(|(_, l)| *l)
}

const CHO: &str = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
const JUNG: &str = "ㅏㅐㅑㅒㅓㅔㅕㅖㅗㅘㅙㅚㅛㅜㅝㅞㅟㅠㅡㅢㅣ";
// Index 0 is "no final consonant".
const JONG: &str = " ㄱㄲㄳㄴㄵㄶㄷㄹㄺㄻㄼㄽㄾㄿㅀㅁㅂㅄㅅㅆㅇㅈㅊㅋㅌㅍㅎ";
const VOWEL_PAIRS: [(char, char, char); 7] = [('ㅗ', 'ㅏ', 'ㅘ'), ('ㅗ', 'ㅐ', 'ㅙ'), ('ㅗ', 'ㅣ', 'ㅚ'), ('ㅜ', 'ㅓ', 'ㅝ'), ('ㅜ', 'ㅔ', 'ㅞ'), ('ㅜ', 'ㅣ', 'ㅟ'), ('ㅡ', 'ㅣ', 'ㅢ')];
const FINAL_PAIRS: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'), ('ㄴ', 'ㅈ', 'ㄵ'), ('ㄴ', 'ㅎ', 'ㄶ'), ('ㄹ', 'ㄱ', 'ㄺ'), ('ㄹ', 'ㅁ', 'ㄻ'), ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'), ('ㄹ', 'ㅌ', 'ㄾ'), ('ㄹ', 'ㅍ', 'ㄿ'), ('ㄹ', 'ㅎ', 'ㅀ'), ('ㅂ', 'ㅅ', 'ㅄ'),
];

fn index(set: &str, c: char) -> Option<usize> {
    set.chars().position(|x| x == c)
}
fn is_vowel(j: char) -> bool {
    index(JUNG, j).is_some()
}
fn pair(pairs: &[(char, char, char)], a: char, b: char) -> Option<char> {
    pairs.iter().find(|p| p.0 == a && p.1 == b).map(|p| p.2)
}

/// What to do on the screen: delete `back` characters, then type `text`.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct Edit {
    pub back: usize,
    pub text: String,
}

/// The syllable being typed (shown on the screen as one character, once something is in it).
#[derive(Default, Debug)]
pub struct Composer {
    cho: Option<char>,
    jung: Option<char>,
    jong: Option<char>,
}

impl Composer {
    /// Any other key, a click or a pause in typing ends the syllable: the next jamo starts anew.
    pub fn reset(&mut self) {
        *self = Composer::default();
    }

    fn shown(&self) -> String {
        match (self.cho, self.jung) {
            (Some(c), Some(v)) => {
                let (l, v) = (index(CHO, c).unwrap_or(0), index(JUNG, v).unwrap_or(0));
                let t = self.jong.and_then(|t| index(JONG, t)).unwrap_or(0);
                char::from_u32(0xac00 + ((l * 21 + v) * 28 + t) as u32).map(String::from).unwrap_or_default()
            }
            (Some(c), None) => c.to_string(),
            (None, Some(v)) => v.to_string(),
            (None, None) => String::new(),
        }
    }

    /// One more jamo: how the screen changes.
    pub fn feed(&mut self, j: char) -> Edit {
        let before = self.shown();
        let mut done = String::new(); // syllables this jamo finished
        if is_vowel(j) {
            match (self.cho, self.jung, self.jong) {
                (Some(_), None, _) => self.jung = Some(j),
                (_, Some(v), None) if pair(&VOWEL_PAIRS, v, j).is_some() => self.jung = pair(&VOWEL_PAIRS, v, j),
                (Some(_), Some(_), Some(t)) => {
                    // The final consonant (or its second half) starts the next syllable: 달 + ㅏ → 다라.
                    let (keep, moved) = FINAL_PAIRS.iter().find(|p| p.2 == t).map_or((None, t), |p| (Some(p.0), p.1));
                    self.jong = keep;
                    done = self.shown();
                    *self = Composer { cho: Some(moved), jung: Some(j), jong: None };
                }
                _ => {
                    done = self.shown();
                    *self = Composer { cho: None, jung: Some(j), jong: None };
                }
            }
        } else {
            match (self.cho, self.jung, self.jong) {
                (Some(_), Some(_), None) if index(JONG, j).is_some() => self.jong = Some(j),
                (Some(_), Some(_), Some(t)) if pair(&FINAL_PAIRS, t, j).is_some() => self.jong = pair(&FINAL_PAIRS, t, j),
                _ => {
                    done = self.shown();
                    self.reset();
                    if index(CHO, j).is_some() {
                        self.cho = Some(j);
                    } else {
                        done.push(j); // ㄳ and the like on their own: nothing to build on
                    }
                }
            }
        }
        let text = done + &self.shown();
        // Unchanged characters at the start stay on the screen.
        match text.strip_prefix(before.as_str()) {
            Some(rest) if !before.is_empty() => Edit { back: 0, text: rest.to_string() },
            _ => Edit { back: usize::from(!before.is_empty()), text },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type jamo into a pretend text field: what it shows after each one.
    fn typed(jamo: &str) -> Vec<String> {
        let (mut c, mut field) = (Composer::default(), String::new());
        jamo.chars()
            .map(|j| {
                let e = c.feed(j);
                for _ in 0..e.back {
                    field.pop();
                }
                field.push_str(&e.text);
                field.clone()
            })
            .collect()
    }
    fn last(jamo: &str) -> String {
        typed(jamo).pop().unwrap_or_default()
    }

    #[test]
    fn syllables_build_up_as_typed() {
        assert_eq!(typed("ㅎㅏㄴㄱㅡㄹ"), ["ㅎ", "하", "한", "한ㄱ", "한그", "한글"]);
        assert_eq!(last("ㅇㅣㅅㄷㅏ"), "잇다");
        assert_eq!(last("ㄱㅗㅏㅇ"), "광");
        assert_eq!(last("ㅇㅡㅣㅅㅏ"), "의사");
    }

    #[test]
    fn a_final_consonant_moves_to_the_next_syllable() {
        assert_eq!(typed("ㄷㅏㄹㅏ"), ["ㄷ", "다", "달", "다라"]);
        assert_eq!(last("ㅇㅏㄴㅎㅇㅏ"), "않아");
        assert_eq!(last("ㅇㅣㄹㄱㅣ"), "일기"); // ㄺ + ㅣ → 일 + 기
        assert_eq!(last("ㄱㅏㅂㅅㅇㅣ"), "값이");
        assert_eq!(last("ㄱㅏㅂㅅㅣ"), "갑시"); // ㅄ + ㅣ → 갑 + 시
    }

    #[test]
    fn what_cannot_join_stays_apart() {
        assert_eq!(last("ㄱㄱ"), "ㄱㄱ");
        assert_eq!(last("ㅏㅏ"), "ㅏㅏ");
        assert_eq!(last("ㄸㅏㄸ"), "따ㄸ"); // ㄸ is never a final consonant
        assert_eq!(last("ㅏㄴ"), "ㅏㄴ");
        assert_eq!(last("ㄳ"), "ㄳ");
        assert_eq!(last("ㅂㅂㅏ"), "ㅂ바");
    }

    #[test]
    fn reset_starts_a_new_syllable() {
        let mut c = Composer::default();
        c.feed('ㅎ');
        c.feed('ㅏ');
        c.reset();
        assert_eq!(c.feed('ㄴ'), Edit { back: 0, text: "ㄴ".into() });
        assert_eq!(c.feed('ㅏ'), Edit { back: 1, text: "나".into() });
    }

    #[test]
    fn keysyms_and_keys() {
        assert_eq!(jamo(0x0100_314e), Some('ㅎ'));
        assert_eq!(jamo(0x0ebe), Some('ㅎ'));
        assert_eq!(jamo(0x0ebf), Some('ㅏ'));
        assert_eq!(jamo(0x0ed3), Some('ㅣ'));
        assert_eq!(jamo(0x0100_ac00), None); // a whole syllable (a phone keyboard) types as it is
        assert_eq!(jamo('g' as u32), None);
        assert_eq!(latin('ㅊ'), Some('c'));
        assert_eq!(latin('ㄲ'), Some('r'));
        assert_eq!(latin('ㅘ'), None);
    }
}
