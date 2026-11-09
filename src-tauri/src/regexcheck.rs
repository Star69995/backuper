//! Live check of a regex filter for the rule editor: syntax errors in Hebrew, a plain-language
//! reading of the pattern (parsed with the same parser the `regex` crate uses, so the reading
//! can't drift from what the engine really does), and a try-out against a sample name.

use crate::engine::native::compile_regex;
use crate::model::{RegexCheck, RegexPart};
use regex_syntax::ast::{
    self, Assertion, AssertionKind, Ast, ClassBracketed, ClassPerl, ClassPerlKind, ClassSet, ClassSetItem, ErrorKind, Group, GroupKind,
    Repetition, RepetitionKind, RepetitionRange, Span,
};

/// Left-to-right isolate, so a pattern fragment inside Hebrew text keeps its own order.
const LRI: char = '\u{2066}';
const PDI: char = '\u{2069}';

/// Why the pattern can't be used (Hebrew, with what to do), or `None` when it compiles.
pub fn error(pattern: &str) -> Option<String> {
    match ast::parse::Parser::new().parse(pattern) {
        Err(e) => Some(parse_error(pattern, &e)),
        Ok(_) => compile_regex(pattern).err().map(|e| {
            if e.contains("size limit") {
                "הביטוי מורכב מדי. כדאי לקצר אותו או לפשט חזרות".to_string()
            } else {
                format!("הביטוי לא נתמך ({})", e.lines().last().unwrap_or(&e).trim())
            }
        }),
    }
}

pub fn check(pattern: &str, sample: Option<&str>) -> RegexCheck {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return RegexCheck::default();
    }
    if let Some(e) = error(pattern) {
        return RegexCheck { error: Some(e), ..Default::default() };
    }
    // Both calls succeed once `error` found nothing.
    let Ok(tree) = ast::parse::Parser::new().parse(pattern) else { return RegexCheck::default() };
    let parts = match &tree {
        Ast::Concat(c) => items(pattern, &c.asts),
        other => vec![(pattern.to_string(), describe(pattern, other))],
    };
    let mut out = RegexCheck {
        parts: parts.into_iter().map(|(token, meaning)| RegexPart { token, meaning }).collect(),
        note: unanchored_note(&tree),
        ..Default::default()
    };
    if let (Some(s), Ok(re)) = (sample.filter(|s| !s.is_empty()), compile_regex(pattern)) {
        let m = re.find(s);
        out.matched = Some(m.is_some());
        out.matched_text = m.map(|m| m.as_str().to_string());
    }
    out
}

fn parse_error(pattern: &str, e: &ast::Error) -> String {
    let what = match e.kind() {
        ErrorKind::GroupUnclosed => "סוגריים ( שלא נסגרו. חסר ) בהמשך".to_string(),
        ErrorKind::GroupUnopened => "סוגריים ) בלי סוגריים פותחים (. כדי לכתוב את הסימן עצמו, כתבו \\)".to_string(),
        ErrorKind::ClassUnclosed => "סוגריים מרובעים [ שלא נסגרו. חסר ] בהמשך".to_string(),
        ErrorKind::RepetitionMissing => "הסימן * + ? או {} בלי תו לפניו. בביטוי רגולרי הם חוזרים על התו שלפניהם, ולא מייצגים \"כל טקסט\" כמו בכלל הרגיל. \
             כדי לדלג על סיומת (למשל *.tmp) עדיף הכלל \"סוגי קבצים ושמות\""
            .to_string(),
        ErrorKind::EscapeUnrecognized => "הסימן \\ מוגדר לפני תו שאין לו משמעות מיוחדת. כדי לכתוב לוכסן הפוך עצמו כתבו \\\\".to_string(),
        ErrorKind::EscapeUnexpectedEof => "הביטוי נגמר ב-\\ בלי תו אחריו".to_string(),
        ErrorKind::ClassRangeInvalid => "טווח לא תקין בסוגריים מרובעים. התו הראשון בטווח צריך להיות קטן מהאחרון (למשל a-z)".to_string(),
        ErrorKind::RepetitionCountInvalid | ErrorKind::RepetitionCountDecimalEmpty | ErrorKind::DecimalInvalid | ErrorKind::DecimalEmpty => {
            "מספר חזרות לא תקין בתוך {}. הצורה הנכונה: {3} או {2,5}".to_string()
        }
        ErrorKind::RepetitionCountUnclosed => "סוגריים מסולסלים { שלא נסגרו".to_string(),
        ErrorKind::UnsupportedLookAround => "הסתכלות קדימה או אחורה, כמו (?=...) ו-(?!...), לא נתמכת".to_string(),
        ErrorKind::UnsupportedBackreference => "הפניה לקבוצה קודמת (\\1) לא נתמכת".to_string(),
        ErrorKind::NestLimitExceeded(_) => "יותר מדי סוגריים בתוך סוגריים".to_string(),
        other => format!("תחביר לא תקין ({other})"),
    };
    let at = pattern[..e.span().start.offset].chars().count() + 1;
    format!("{what} (במקום ה-{at} בביטוי)")
}

fn text<'a>(p: &'a str, span: &Span) -> &'a str {
    &p[span.start.offset..span.end.offset]
}

/// A literal character for display (a space would be invisible).
fn lit(c: char) -> String {
    match c {
        ' ' => "רווח".into(),
        '\t' => "טאב".into(),
        c => format!("{LRI}\"{c}\"{PDI}"),
    }
}

fn quote(s: &str) -> String {
    if s.chars().count() == 1 {
        lit(s.chars().next().unwrap_or(' '))
    } else {
        format!("{LRI}\"{s}\"{PDI}")
    }
}

/// The parts of a sequence, with runs of plain characters merged into one text.
fn items(p: &str, asts: &[Ast]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < asts.len() {
        if matches!(asts[i], Ast::Literal(_)) {
            let mut j = i;
            let mut s = String::new();
            while let Some(Ast::Literal(l)) = asts.get(j) {
                s.push(l.c);
                j += 1;
            }
            let token = &p[asts[i].span().start.offset..asts[j - 1].span().end.offset];
            let meaning = if j - i == 1 { format!("התו {}", quote(&s)) } else { format!("הטקסט {}", quote(&s)) };
            out.push((token.to_string(), meaning));
            i = j;
        } else {
            out.push((text(p, asts[i].span()).to_string(), describe(p, &asts[i])));
            i += 1;
        }
    }
    out
}

/// The text of an AST made only of plain characters.
fn plain_text(a: &Ast) -> Option<String> {
    match a {
        Ast::Literal(l) => Some(l.c.to_string()),
        Ast::Concat(c) => c.asts.iter().map(plain_text).collect(),
        _ => None,
    }
}

fn describe(p: &str, a: &Ast) -> String {
    match a {
        Ast::Empty(_) => "כלום".into(),
        Ast::Flags(f) => format!("שינוי הגדרות ({})", text(p, &f.span)),
        Ast::Literal(l) => format!("התו {}", lit(l.c)),
        Ast::Dot(_) => "תו כלשהו (אחד)".into(),
        Ast::Assertion(x) => assertion(x).into(),
        Ast::ClassUnicode(c) => format!("תו מקבוצת Unicode {LRI}{}{PDI}", text(p, &c.span)),
        Ast::ClassPerl(c) => perl(c).into(),
        Ast::ClassBracketed(c) => bracketed(p, c),
        Ast::Repetition(r) => repetition(p, r),
        Ast::Group(g) => group(p, g),
        Ast::Alternation(alt) => {
            let texts: Option<Vec<String>> = alt.asts.iter().map(plain_text).collect();
            match texts {
                Some(t) => format!("אחד מהטקסטים: {}", t.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ")),
                None => format!("אחד מאלה: {}", alt.asts.iter().map(|x| describe(p, x)).collect::<Vec<_>>().join(" או ")),
            }
        }
        Ast::Concat(c) => items(p, &c.asts).into_iter().map(|(_, m)| m).collect::<Vec<_>>().join(", ואז "),
    }
}

fn assertion(a: &Assertion) -> &'static str {
    match a.kind {
        AssertionKind::StartLine | AssertionKind::StartText => "תחילת השם",
        AssertionKind::EndLine | AssertionKind::EndText => "סוף השם",
        AssertionKind::NotWordBoundary => "מקום שאינו גבול של מילה",
        _ => "גבול של מילה",
    }
}

fn perl(c: &ClassPerl) -> &'static str {
    match (&c.kind, c.negated) {
        (ClassPerlKind::Digit, false) => "ספרה",
        (ClassPerlKind::Digit, true) => "תו שאינו ספרה",
        (ClassPerlKind::Word, false) => "אות, ספרה או _",
        (ClassPerlKind::Word, true) => "תו שאינו אות, ספרה או _",
        (ClassPerlKind::Space, false) => "רווח (או טאב)",
        (ClassPerlKind::Space, true) => "תו שאינו רווח",
    }
}

fn bracketed(p: &str, c: &ClassBracketed) -> String {
    let inner = match &c.kind {
        ClassSet::Item(item) => class_item(p, item),
        ClassSet::BinaryOp(op) => format!("{LRI}{}{PDI}", text(p, &op.span)),
    };
    format!("{}: {inner}", if c.negated { "כל תו חוץ מ" } else { "אחד מהתווים" })
}

fn class_item(p: &str, item: &ClassSetItem) -> String {
    match item {
        ClassSetItem::Empty(_) => String::new(),
        ClassSetItem::Literal(l) => lit(l.c),
        ClassSetItem::Range(r) => format!("{} עד {}", lit(r.start.c), lit(r.end.c)),
        ClassSetItem::Ascii(a) => format!("{LRI}{}{PDI}", text(p, &a.span)),
        ClassSetItem::Unicode(u) => format!("{LRI}{}{PDI}", text(p, &u.span)),
        ClassSetItem::Perl(c) => perl(c).into(),
        ClassSetItem::Bracketed(b) => format!("({})", bracketed(p, b)),
        ClassSetItem::Union(u) => u.items.iter().map(|i| class_item(p, i)).collect::<Vec<_>>().join(", "),
    }
}

fn repetition(p: &str, r: &Repetition) -> String {
    let count = match &r.op.kind {
        RepetitionKind::ZeroOrOne => "לא חובה".to_string(),
        RepetitionKind::ZeroOrMore => "אפס פעמים או יותר".to_string(),
        RepetitionKind::OneOrMore => "פעם אחת או יותר".to_string(),
        RepetitionKind::Range(RepetitionRange::Exactly(n)) => format!("בדיוק {n} פעמים"),
        RepetitionKind::Range(RepetitionRange::AtLeast(n)) => format!("{n} פעמים או יותר"),
        RepetitionKind::Range(RepetitionRange::Bounded(a, b)) => format!("בין {a} ל-{b} פעמים"),
    };
    let lazy = if r.greedy { "" } else { " (הכי מעט שאפשר)" };
    format!("{count}: {}{lazy}", describe(p, &r.ast))
}

fn group(p: &str, g: &Group) -> String {
    let inner = describe(p, &g.ast);
    match &g.kind {
        GroupKind::CaptureName { name, .. } => format!("קבוצה בשם {LRI}{}{PDI}: {inner}", name.name),
        _ => inner,
    }
}

/// Without `^` / `$` the pattern may match anywhere in the name, which surprises people.
fn unanchored_note(tree: &Ast) -> Option<String> {
    let anchor = |a: &Ast, start: bool| {
        matches!(a, Ast::Assertion(x) if matches!(
            (&x.kind, start),
            (AssertionKind::StartLine | AssertionKind::StartText, true) | (AssertionKind::EndLine | AssertionKind::EndText, false)
        ))
    };
    let (starts, ends) = match tree {
        Ast::Alternation(_) => return None,
        Ast::Concat(c) => (c.asts.first().is_some_and(|a| anchor(a, true)), c.asts.last().is_some_and(|a| anchor(a, false))),
        other => (anchor(other, true), anchor(other, false)),
    };
    (!starts && !ends).then(|| {
        "בלי ^ בהתחלה ו-$ בסוף, מספיק שהביטוי יתאים לחלק כלשהו מהשם (למשל copy מתאים גם ל-my copy.txt). \
         כדי להתאים לשם כולו, התחילו ב-^ וסיימו ב-$."
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meanings(p: &str) -> Vec<String> {
        check(p, None).parts.into_iter().map(|x| x.meaning).collect()
    }

    #[test]
    fn leading_dot() {
        let c = check(r"^\.", Some(".gitignore"));
        assert!(c.error.is_none());
        assert_eq!(c.parts.iter().map(|x| x.token.as_str()).collect::<Vec<_>>(), ["^", r"\."]);
        assert_eq!(meanings(r"^\.")[0], "תחילת השם");
        assert_eq!(c.matched, Some(true));
        assert_eq!(c.matched_text.as_deref(), Some("."));
        assert_eq!(check(r"^\.", Some("a.txt")).matched, Some(false));
        assert!(c.note.is_none());
    }

    #[test]
    fn explains_common_patterns() {
        let m = meanings(r"^IMG_\d+");
        assert_eq!(m.len(), 3);
        assert!(m[1].starts_with("הטקסט") && m[1].contains("IMG_"));
        assert!(m[2].starts_with("פעם אחת או יותר") && m[2].contains("ספרה"));
        assert!(meanings("^(build|cache)$")[1].starts_with("אחד מהטקסטים"));
        assert!(meanings("[^a-c0-9]")[0].starts_with("כל תו חוץ מ"));
        assert!(meanings("a{2,4}?")[0].contains("הכי מעט שאפשר"));
    }

    #[test]
    fn explains_every_construct_without_panicking() {
        for p in [
            r"(?i)a", r"(?P<n>a)\b\B", r"[[:alpha:]\d\p{L}]", r"\pL+", r"[a-z&&[^c]]", r"a|b|c.", r"(?:x|yz)*", r"\x41\u{1F600}", "()", "a||b", "[ ]\t",
        ] {
            let c = check(p, Some("a"));
            assert!(c.error.is_none() && !c.parts.is_empty(), "{p}: {c:?}");
        }
    }

    #[test]
    fn reports_errors_in_hebrew() {
        for p in ["(", ")", "[a", "*.jpg", r"\", "a{2", "(?=a)", r"(a)\1", "[z-a]", "a{x}"] {
            let e = error(p).unwrap_or_else(|| panic!("{p} should be invalid"));
            assert!(e.contains("במקום ה-") || e.contains("לא נתמכ"), "{p}: {e}");
            assert!(check(p, Some("a")).parts.is_empty());
        }
        assert!(error("^IMG_\\d+").is_none());
        assert!(error("").is_none());
    }

    #[test]
    fn warns_about_unanchored_patterns() {
        assert!(check("copy", None).note.is_some());
        assert!(check("^copy", None).note.is_none());
        assert!(check("copy$", None).note.is_none());
        assert!(check("a|b", None).note.is_none());
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(check("^img", Some("IMG_1.jpg")).matched, Some(true));
        assert_eq!(check("^img", None).matched, None);
    }
}
