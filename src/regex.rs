//! A tiny regular-expression engine for search.
//!
//! Koda keeps its dependency surface small, so this implements the subset of
//! regular-expression syntax that is genuinely useful when searching code,
//! rather than pulling in a full engine:
//!
//! * literals and `.` (any character)
//! * greedy `*`, `+` and `?` quantifiers
//! * character classes `[a-z]`, `[^0-9]`, including escapes inside them
//! * anchors `^` (start of line) and `$` (end of line)
//! * escapes `\d \D \w \W \s \S`, plus escaping punctuation
//!
//! Groups, alternation and `{n,m}` repetition are deliberately unsupported and
//! report a clear error instead of matching the wrong thing. Matching is
//! line-oriented and positions are measured in **characters**, matching the
//! rest of the editor.

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    atoms: Vec<Atom>,
}

#[derive(Clone, Debug)]
struct Atom {
    matcher: Matcher,
    repeat: Repeat,
}

#[derive(Clone, Debug)]
enum Matcher {
    Literal(char),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Digit(bool),
    Word(bool),
    Space(bool),
    Start,
    End,
}

#[derive(Clone, Copy, Debug)]
enum ClassItem {
    Single(char),
    Range(char, char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Repeat {
    One,
    Optional,
    ZeroOrMore,
    OneOrMore,
}

/// Cap on repetitions of a single atom, so a pathological pattern cannot spin.
const MAX_REPEAT: usize = 10_000;

impl Regex {
    /// Compile `pattern`, returning a readable message on unsupported syntax.
    pub fn new(pattern: &str) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut atoms: Vec<Atom> = Vec::new();
        let mut i = 0;

        while i < chars.len() {
            let (matcher, consumed) = parse_atom(&chars, i)?;
            i += consumed;

            let repeat = match chars.get(i) {
                Some('*') => {
                    i += 1;
                    Repeat::ZeroOrMore
                }
                Some('+') => {
                    i += 1;
                    Repeat::OneOrMore
                }
                Some('?') => {
                    i += 1;
                    Repeat::Optional
                }
                _ => Repeat::One,
            };
            if matches!(matcher, Matcher::Start | Matcher::End) && repeat != Repeat::One {
                return Err("anchors cannot be repeated".to_string());
            }
            atoms.push(Atom { matcher, repeat });
        }

        if atoms.is_empty() {
            return Err("empty pattern".to_string());
        }
        Ok(Regex { atoms })
    }

    /// Find the first match at or after character index `from`.
    ///
    /// Returns `(start, end)` character offsets.
    pub fn find(
        &self,
        chars: &[char],
        from: usize,
        case_insensitive: bool,
    ) -> Option<(usize, usize)> {
        let mut start = from;
        while start <= chars.len() {
            if let Some(end) = self.match_from(chars, start, case_insensitive) {
                return Some((start, end));
            }
            start += 1;
        }
        None
    }

    /// Whether the pattern matches the text anywhere.
    pub fn is_match(&self, chars: &[char], case_insensitive: bool) -> bool {
        self.find(chars, 0, case_insensitive).is_some()
    }

    fn match_from(&self, chars: &[char], start: usize, ci: bool) -> Option<usize> {
        self.match_atoms(chars, 0, start, ci)
    }

    fn match_atoms(&self, chars: &[char], index: usize, pos: usize, ci: bool) -> Option<usize> {
        let Some(atom) = self.atoms.get(index) else {
            return Some(pos);
        };
        let rest = |p: usize| self.match_atoms(chars, index + 1, p, ci);

        match atom.repeat {
            Repeat::One => {
                let next = match_matcher(&atom.matcher, chars, pos, ci)?;
                rest(next)
            }
            Repeat::Optional => {
                // Greedy: try one occurrence before falling back to none.
                if let Some(next) = match_matcher(&atom.matcher, chars, pos, ci)
                    && let Some(end) = rest(next)
                {
                    return Some(end);
                }
                rest(pos)
            }
            Repeat::ZeroOrMore | Repeat::OneOrMore => {
                let minimum = if atom.repeat == Repeat::OneOrMore {
                    1
                } else {
                    0
                };
                let mut positions = vec![pos];
                let mut cursor = pos;
                while positions.len() <= MAX_REPEAT {
                    let Some(next) = match_matcher(&atom.matcher, chars, cursor, ci) else {
                        break;
                    };
                    if next == cursor {
                        break; // an empty match would loop forever
                    }
                    positions.push(next);
                    cursor = next;
                }
                // Greedy: attempt the longest run first.
                for &point in positions.iter().skip(minimum).rev() {
                    if let Some(end) = rest(point) {
                        return Some(end);
                    }
                }
                None
            }
        }
    }
}

/// Parse one atom starting at `index`, returning it and how many chars it used.
fn parse_atom(chars: &[char], index: usize) -> Result<(Matcher, usize), String> {
    let c = chars[index];
    match c {
        '(' | ')' | '|' | '{' | '}' => Err(format!(
            "`{c}` is not supported (no groups, alternation or repetition braces)"
        )),
        '[' => parse_class(chars, index),
        '\\' => {
            let Some(&escaped) = chars.get(index + 1) else {
                return Err("dangling `\\`".to_string());
            };
            let matcher = match escaped {
                'd' => Matcher::Digit(false),
                'D' => Matcher::Digit(true),
                'w' => Matcher::Word(false),
                'W' => Matcher::Word(true),
                's' => Matcher::Space(false),
                'S' => Matcher::Space(true),
                'n' => Matcher::Literal('\n'),
                't' => Matcher::Literal('\t'),
                other => Matcher::Literal(other),
            };
            Ok((matcher, 2))
        }
        '.' => Ok((Matcher::Any, 1)),
        '^' => Ok((Matcher::Start, 1)),
        '$' => Ok((Matcher::End, 1)),
        other => Ok((Matcher::Literal(other), 1)),
    }
}

/// Parse a `[...]` character class starting at the opening bracket.
fn parse_class(chars: &[char], index: usize) -> Result<(Matcher, usize), String> {
    let mut i = index + 1;
    let negated = matches!(chars.get(i), Some('^'));
    if negated {
        i += 1;
    }
    let mut items = Vec::new();
    let mut closed = false;

    while i < chars.len() {
        if chars[i] == ']' && !items.is_empty() {
            closed = true;
            i += 1;
            break;
        }
        let (first, used) = class_char(chars, i)?;
        // A dash that is not the last character forms a range.
        if chars.get(i + used) == Some(&'-') && chars.get(i + used + 1).is_some_and(|&c| c != ']') {
            let (second, second_used) = class_char(chars, i + used + 1)?;
            items.push(ClassItem::Range(first, second));
            i += used + 1 + second_used;
        } else {
            items.push(ClassItem::Single(first));
            i += used;
        }
    }

    if !closed {
        return Err("unclosed `[`".to_string());
    }
    Ok((Matcher::Class { negated, items }, i - index))
}

/// Read one character (or escape) inside a class, returning it and the chars used.
fn class_char(chars: &[char], index: usize) -> Result<(char, usize), String> {
    if chars[index] == '\\' {
        let Some(&escaped) = chars.get(index + 1) else {
            return Err("dangling `\\` in class".to_string());
        };
        return Ok((escaped, 2));
    }
    Ok((chars[index], 1))
}

/// Match a single atom (ignoring its quantifier) at `pos`.
fn match_matcher(matcher: &Matcher, chars: &[char], pos: usize, ci: bool) -> Option<usize> {
    match matcher {
        Matcher::Start => (pos == 0).then_some(pos),
        Matcher::End => (pos == chars.len()).then_some(pos),
        Matcher::Any => chars.get(pos).map(|_| pos + 1),
        Matcher::Literal(expected) => {
            let c = *chars.get(pos)?;
            chars_equal(c, *expected, ci).then_some(pos + 1)
        }
        Matcher::Digit(negated) => match_char(chars, pos, |c| c.is_ascii_digit(), *negated),
        Matcher::Word(negated) => {
            match_char(chars, pos, |c| c.is_alphanumeric() || c == '_', *negated)
        }
        Matcher::Space(negated) => match_char(chars, pos, char::is_whitespace, *negated),
        Matcher::Class { negated, items } => {
            let c = *chars.get(pos)?;
            let mut hit = class_matches(items, c);
            if !hit && ci {
                hit = c
                    .to_lowercase()
                    .next()
                    .is_some_and(|l| class_matches(items, l))
                    || c.to_uppercase()
                        .next()
                        .is_some_and(|u| class_matches(items, u));
            }
            (hit != *negated).then_some(pos + 1)
        }
    }
}

fn match_char(
    chars: &[char],
    pos: usize,
    predicate: fn(char) -> bool,
    negated: bool,
) -> Option<usize> {
    let c = *chars.get(pos)?;
    (predicate(c) != negated).then_some(pos + 1)
}

fn class_matches(items: &[ClassItem], c: char) -> bool {
    items.iter().any(|item| match *item {
        ClassItem::Single(single) => single == c,
        ClassItem::Range(from, to) => from <= c && c <= to,
    })
}

fn chars_equal(a: char, b: char, ci: bool) -> bool {
    if ci {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(pattern: &str, text: &str) -> Option<(usize, usize)> {
        let regex = Regex::new(pattern).expect("valid pattern");
        let chars: Vec<char> = text.chars().collect();
        regex.find(&chars, 0, false)
    }

    #[test]
    fn literals_and_dot() {
        assert_eq!(find("bc", "abcd"), Some((1, 3)));
        assert_eq!(find("a.c", "xxabc"), Some((2, 5)));
        assert_eq!(find("z", "abcd"), None);
    }

    #[test]
    fn quantifiers_are_greedy() {
        assert_eq!(find("ab*", "abbc"), Some((0, 3)));
        assert_eq!(find("ab+c", "abbc"), Some((0, 4)));
        assert_eq!(find("ab?c", "ac"), Some((0, 2)));
        assert_eq!(find("ab?c", "abc"), Some((0, 3)));
    }

    #[test]
    fn classes_and_negation() {
        assert_eq!(find("[0-9]+", "ab123cd"), Some((2, 5)));
        assert_eq!(find("[^0-9]+", "12ab34"), Some((2, 4)));
        assert_eq!(find("[a-cx]+", "zzabxc"), Some((2, 6)));
    }

    #[test]
    fn anchors() {
        assert_eq!(find("^ab", "abab"), Some((0, 2)));
        assert_eq!(find("ab$", "abab"), Some((2, 4)));
        assert_eq!(find("^b", "ab"), None);
    }

    #[test]
    fn escapes() {
        assert_eq!(find(r"\d+", "abc123"), Some((3, 6)));
        assert_eq!(find(r"\w+", "  hi_there "), Some((2, 10)));
        assert_eq!(find(r"a\.b", "axb a.b"), Some((4, 7)));
    }

    #[test]
    fn case_insensitive_matching() {
        let regex = Regex::new("hello").unwrap();
        let chars: Vec<char> = "say Hello".chars().collect();
        assert_eq!(regex.find(&chars, 0, false), None);
        assert_eq!(regex.find(&chars, 0, true), Some((4, 9)));
    }

    #[test]
    fn unsupported_syntax_is_reported() {
        assert!(Regex::new("(a|b)").is_err());
        assert!(Regex::new("a{2}").is_err());
        assert!(Regex::new("[abc").is_err());
        assert!(Regex::new("").is_err());
    }

    #[test]
    fn backtracking_finds_a_later_match() {
        // `a+aab` must give back the last `a`s to let `b` match.
        assert_eq!(find("a+aab", "aaaab"), Some((0, 5)));
    }
}
