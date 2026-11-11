//! The filter check in the rule editor: one path run through all of a task's filter rules (and
//! the global ones) together, with the engine's own matching (`native::verdict`), plus what each
//! rule does to it. A path that exists is checked with its real size, date, attributes and 8.3 name.

use crate::engine::native::{self, Item, Verdict};
use crate::engine::Filters;
use crate::filters;
use crate::model::{FilterCheck, FilterProbe, FilterRule, OnDisk, RuleEffect, RuleResult};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_DIRECTORY: u32 = 0x10;
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
const TICKS_PER_SEC: u64 = 10_000_000;
const MB: f64 = 1024.0 * 1024.0;

/// Left-to-right isolate, so a path inside Hebrew text keeps its own order.
fn ltr(s: &str) -> String {
    format!("\u{2066}{s}\u{2069}")
}

fn now_ticks() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() * TICKS_PER_SEC + UNIX_EPOCH_TICKS
}

fn rfc3339(filetime: u64) -> Option<String> {
    let secs = (filetime.checked_sub(UNIX_EPOCH_TICKS)? / TICKS_PER_SEC) as i64;
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.to_rfc3339())
}

/// "C:\..." or "\\server\share\...": the root ("C:", "\\server\share") and the rest.
fn split_root(p: &str) -> Option<(&str, &str)> {
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\') {
        return Some((&p[..2], p[2..].trim_start_matches('\\')));
    }
    let unc = p.strip_prefix("\\\\")?;
    let mut parts = unc.splitn(3, '\\');
    let (server, share) = (parts.next()?, parts.next()?);
    if server.is_empty() || share.is_empty() {
        return None;
    }
    let root_len = 2 + server.len() + 1 + share.len();
    Some((&p[..root_len], p[root_len..].trim_start_matches('\\')))
}

/// `path` under `dir` (case-insensitive): the part after it, "" for the folder itself.
fn under<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    let head = path.get(..dir.len())?;
    if head.to_lowercase() != dir.to_lowercase() {
        return None;
    }
    let rest = &path[dir.len()..];
    if rest.is_empty() {
        Some("")
    } else {
        rest.strip_prefix('\\')
    }
}

fn is_full_path_rule(r: &FilterRule) -> bool {
    match r {
        FilterRule::File { .. } => true,
        FilterRule::Folder { value } => value.contains('\\'),
        _ => false,
    }
}

fn is_only(r: &FilterRule) -> bool {
    matches!(r, FilterRule::Include { .. } | FilterRule::IncludeName { .. } | FilterRule::Regex { include: true, .. })
}

fn is_include_names(r: &FilterRule) -> bool {
    matches!(r, FilterRule::Include { .. } | FilterRule::IncludeName { .. })
}

pub fn check(rules: &[FilterRule], global: &[FilterRule], sources: &[String], probe: &FilterProbe) -> FilterCheck {
    let mut out = FilterCheck {
        rules: vec![RuleResult::default(); rules.len()],
        global_rules: vec![RuleResult::default(); global.len()],
        ..Default::default()
    };
    let typed = probe.path.trim().trim_matches('"').trim().replace('/', "\\");
    if typed.is_empty() {
        return out;
    }
    if typed.contains(['*', '?']) {
        out.notes
            .push("כאן בודקים נתיב אחד, בלי * או ?. הקלידו שם של קובץ או תיקייה מסוימים.".into());
        return out;
    }
    let folder_hint = typed.ends_with('\\');
    let path = typed.trim_end_matches('\\');
    let all = || rules.iter().chain(global);

    // Which source the path is in: its base for full-path rules, and where the folders on the way start.
    let sources: Vec<String> = sources
        .iter()
        .map(|s| s.trim().replace('/', "\\").trim_end_matches('\\').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let absolute = split_root(path).is_some();
    let (base, rel) = if let Some((root, rest)) = split_root(path) {
        match sources
            .iter()
            .filter_map(|s| under(path, s).map(|r| (s.clone(), r)))
            .max_by_key(|(s, _)| s.len())
        {
            Some((_, "")) => {
                out.kept = Some(true);
                out.folder = true;
                out.notes
                    .push("זו תיקיית המקור עצמה. הקלידו נתיב של קובץ או תיקייה שבתוכה.".into());
                return out;
            }
            Some((s, r)) => (s, r.to_string()),
            None => {
                if !sources.is_empty() {
                    out.notes
                        .push("הנתיב לא נמצא בתוך אף אחת מתיקיות המקור של המשימה, אז הוא נבדק כאילו מגבים את הכונן כולו.".into());
                }
                (root.to_string(), rest.to_string())
            }
        }
    } else {
        if all().any(is_full_path_rule) {
            out.notes.push(match sources.first() {
                Some(s) => format!(
                    "כללים עם נתיב מלא נבדקו מול {}. כדי לבדוק מקור אחר, הקלידו נתיב מלא.",
                    ltr(&format!("{s}\\{}", path.trim_start_matches('\\')))
                ),
                None => "כללים עם נתיב מלא נבדקים רק מול נתיב מלא (למשל C:\\Users\\...).".into(),
            });
        }
        (
            sources.first().cloned().unwrap_or_default(),
            path.trim_start_matches('\\').to_string(),
        )
    };
    let names: Vec<&str> = rel.split('\\').filter(|n| !n.is_empty()).collect();
    if names.is_empty() {
        out.notes.push("הקלידו נתיב של קובץ או תיקייה, לא רק כונן.".into());
        return out;
    }

    // Each folder on the way, then the item: from disk when they exist.
    let mut parts = Vec::with_capacity(names.len());
    let mut on_disk = None;
    let mut full = base.clone();
    for (i, name) in names.iter().enumerate() {
        full = if full.is_empty() {
            name.to_string()
        } else {
            format!("{full}\\{name}")
        };
        let last = i + 1 == names.len();
        let found = if absolute || !base.is_empty() {
            native::find(Path::new(&full))
        } else {
            None
        };
        let item = match found {
            Some(f) => {
                if last {
                    on_disk = Some(f.clone());
                }
                f
            }
            None if !last => Item {
                name: name.to_string(),
                short_name: String::new(),
                attributes: ATTR_DIRECTORY,
                reparse_tag: 0,
                size: 0,
                modified: 0,
            },
            None => Item {
                name: name.to_string(),
                short_name: String::new(),
                attributes: if folder_hint { ATTR_DIRECTORY } else { 0 }
                    | if probe.hidden { ATTR_HIDDEN } else { 0 }
                    | if probe.system { ATTR_SYSTEM } else { 0 },
                reparse_tag: 0,
                size: probe.size_mb.map_or(0, |m| (m.max(0.0) * MB) as u64),
                modified: now_ticks().saturating_sub(probe.age_days.unwrap_or(0) as u64 * 86_400 * TICKS_PER_SEC),
            },
        };
        parts.push(item);
    }
    let item = &parts[parts.len() - 1];
    out.folder = item.is_dir();
    let is_folder = out.folder;
    out.on_disk = on_disk.map(|d| OnDisk {
        size: d.size,
        modified: rfc3339(d.modified),
        hidden: d.attributes & ATTR_HIDDEN != 0,
        system: d.attributes & ATTR_SYSTEM != 0,
    });

    // Size and date rules on a file that isn't on disk need values the user didn't give.
    let known = out.on_disk.is_some() || out.folder;
    let unknown = |r: &FilterRule| match r {
        FilterRule::LargerThan { .. } if !known && probe.size_mb.is_none() => Some("הקובץ לא נמצא בדיסק - הזינו גודל כדי לבדוק"),
        FilterRule::OlderThan { .. } if !known && probe.age_days.is_none() => {
            Some("הקובץ לא נמצא בדיסק - הזינו לפני כמה ימים שונה")
        }
        _ => None,
    };
    let invalid = |r: &FilterRule| {
        filters::validate(std::slice::from_ref(r))
            .err()
            .map(|e| e.trim_start_matches("כלל סינון:").trim().to_string())
    };
    let usable = |r: &&FilterRule| invalid(r).is_none() && unknown(r).is_none();
    let run = |f: &Filters| native::verdict(&base, &parts, f);

    let builtin = run(&Filters::default());
    let overall = run(&filters::compile(all().filter(usable)));
    let alone = |r: &FilterRule| run(&filters::compile([r]));
    // "Back up only" file-name rules add up: a file another one matches isn't left out by this one.
    let include_hit = all()
        .filter(usable)
        .any(|r| is_include_names(r) && alone(r) == Verdict::Kept);
    let effect = |r: &FilterRule| -> RuleResult {
        if let Some(e) = invalid(r) {
            return RuleResult {
                effect: RuleEffect::Invalid,
                detail: Some(e),
            };
        }
        if let Some(why) = unknown(r) {
            return RuleResult {
                effect: RuleEffect::Unknown,
                detail: Some(why.into()),
            };
        }
        if builtin != Verdict::Kept {
            return RuleResult::default();
        }
        match alone(r) {
            Verdict::Kept if is_only(r) && !is_folder => RuleResult {
                effect: RuleEffect::Keeps,
                detail: None,
            },
            Verdict::Kept | Verdict::Link(_) => RuleResult::default(),
            Verdict::Folder(f) => RuleResult {
                effect: RuleEffect::Skips,
                detail: (f != rel).then(|| format!("מדלג על התיקייה {} שבדרך", ltr(&f))),
            },
            Verdict::File if is_include_names(r) && include_hit => RuleResult::default(),
            Verdict::File => RuleResult {
                effect: RuleEffect::Skips,
                detail: is_only(r).then(|| "הקובץ לא תואם לכלל, וכל מה שלא תואם לא יגובה".to_string()),
            },
        }
    };
    out.rules = rules.iter().map(effect).collect();
    out.global_rules = global.iter().map(effect).collect();

    out.kept = Some(overall == Verdict::Kept);
    out.reason = match (&builtin, &overall) {
        (_, Verdict::Kept) => None,
        (Verdict::Link(f), _) if *f == rel => Some("זה קישור (junction) לתיקייה במקום אחר. הגיבוי לא נכנס לקישורים כאלה.".into()),
        (Verdict::Link(f), _) => Some(format!(
            "התיקייה {} בדרך היא קישור (junction) למקום אחר, והגיבוי לא נכנס לקישורים כאלה.",
            ltr(f)
        )),
        (Verdict::Folder(f), _) => Some(format!("התיקייה {} אף פעם לא מגובה (תיקייה של Windows).", ltr(f))),
        (_, Verdict::Folder(f)) if *f == rel => Some("כלל סינון מדלג על התיקייה, יחד עם כל מה שבתוכה.".into()),
        (_, Verdict::Folder(f)) => Some(format!("היא בתוך התיקייה {}, שכלל סינון מדלג עליה.", ltr(f))),
        _ => Some("כלל סינון מדלג על הקובץ.".into()),
    };
    if out.folder && all().any(|r| !matches!(r, FilterRule::Folder { .. } | FilterRule::Regex { folders: true, .. })) {
        out.notes
            .push("בתיקייה נבדקים רק כללי התיקיות. כללים על קבצים חלים על כל קובץ בתוכה בנפרד.".into());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FilterRule as R;

    fn probe(path: &str) -> FilterProbe {
        FilterProbe {
            path: path.into(),
            ..Default::default()
        }
    }

    fn effects(c: &FilterCheck) -> Vec<RuleEffect> {
        c.rules.iter().map(|r| r.effect).collect()
    }

    #[test]
    fn all_rules_together() {
        let rules = [
            R::Extension { value: "tmp".into() },
            R::Folder {
                value: "node_modules".into(),
            },
            R::Regex {
                value: r"^IMG_\d+".into(),
                include: false,
                folders: false,
            },
            R::Regex {
                value: "^build".into(),
                include: false,
                folders: true,
            },
        ];
        let src = vec![r"Z:\nowhere\src".to_string()];
        let c = check(&rules, &[], &src, &probe(r"Z:\nowhere\src\a\node_modules\x\IMG_12.jpg"));
        assert_eq!(c.kept, Some(false));
        assert_eq!(
            effects(&c),
            [RuleEffect::None, RuleEffect::Skips, RuleEffect::Skips, RuleEffect::None]
        );
        assert!(c.rules[1].detail.as_deref().is_some_and(|d| d.contains(r"a\node_modules")));
        let c = check(&rules, &[], &src, &probe(r"docs\report.docx"));
        assert_eq!(c.kept, Some(true));
        assert!(effects(&c).iter().all(|e| *e == RuleEffect::None));
        let c = check(&rules, &[], &src, &probe(r"build2\report.docx"));
        assert_eq!((c.kept, c.rules[3].effect), (Some(false), RuleEffect::Skips));
        let c = check(&rules, &[], &src, &probe(r"build2\"));
        assert!(c.folder && c.kept == Some(false) && c.rules[3].detail.is_none());
    }

    #[test]
    fn only_rules_add_up() {
        let rules = [R::Include { value: "docx".into() }, R::Include { value: "xlsx".into() }];
        let c = check(&rules, &[], &[], &probe("a.xlsx"));
        assert_eq!(c.kept, Some(true));
        assert_eq!(effects(&c), [RuleEffect::None, RuleEffect::Keeps]);
        let c = check(&rules, &[], &[], &probe("a.pdf"));
        assert_eq!(c.kept, Some(false));
        assert_eq!(effects(&c), [RuleEffect::Skips, RuleEffect::Skips]);
    }

    #[test]
    fn global_rules_and_unknown_values() {
        let rules = [
            R::LargerThan { mb: 100 },
            R::Regex {
                value: "(".into(),
                include: false,
                folders: false,
            },
        ];
        let global = [
            R::Hidden,
            R::File {
                value: r"Z:\nowhere\src\big.bin".into(),
            },
        ];
        let src = vec![r"Z:\nowhere\src".to_string()];
        let c = check(&rules, &global, &src, &probe("big.bin"));
        assert_eq!(c.kept, Some(false));
        assert_eq!(effects(&c), [RuleEffect::Unknown, RuleEffect::Invalid]);
        assert_eq!(
            c.global_rules.iter().map(|r| r.effect).collect::<Vec<_>>(),
            [RuleEffect::None, RuleEffect::Skips]
        );
        let c = check(
            &rules,
            &global,
            &src,
            &FilterProbe {
                path: "small.bin".into(),
                size_mb: Some(500.0),
                hidden: true,
                ..Default::default()
            },
        );
        assert_eq!(c.kept, Some(false));
        assert_eq!(c.rules[0].effect, RuleEffect::Skips);
        assert_eq!(c.global_rules[0].effect, RuleEffect::Skips);
    }

    #[test]
    fn always_skipped_folders_and_odd_paths() {
        let c = check(
            &[R::Extension { value: "tmp".into() }],
            &[],
            &[],
            &probe(r"$RECYCLE.BIN\x.tmp"),
        );
        assert_eq!((c.kept, c.rules[0].effect), (Some(false), RuleEffect::None));
        assert!(c.reason.is_some());
        assert_eq!(check(&[], &[], &[], &probe("*.tmp")).kept, None);
        assert_eq!(check(&[], &[], &[], &probe(r"C:\")).kept, None);
        assert_eq!(check(&[], &[], &[r"C:\src".into()], &probe(r"c:\SRC\")).kept, Some(true));
    }
}
