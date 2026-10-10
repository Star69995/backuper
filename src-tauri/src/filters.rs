//! User filter rules -> engine filters.

use crate::engine::native::compile_regex;
use crate::engine::Filters;
use crate::model::FilterRule;

/// "lrcat, *.docx;notes.txt" -> ["*.lrcat", "*.docx", "notes.txt"]. Used by both "only" and "skip"
/// rules on file types/names. A bare word or ".ext" is an
/// extension; anything with a wildcard or a dot inside is used as typed (names may contain spaces).
pub fn patterns(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            if p.contains(['*', '?']) {
                p.to_string()
            } else if let Some(ext) = p.strip_prefix('.') {
                format!("*.{ext}")
            } else if !p.contains('.') {
                format!("*.{p}")
            } else {
                p.to_string()
            }
        })
        .collect()
}

pub fn validate(rules: &[FilterRule]) -> Result<(), String> {
    for r in rules {
        match r {
            FilterRule::Include { value } if patterns(value).is_empty() => {
                return Err("כלל סינון: יש להזין אילו קבצים לגבות (למשל *.lrcat)".into())
            }
            FilterRule::Include { value } if value.contains('\\') => {
                return Err(format!("כלל סינון: תבנית שם קובץ לא יכולה לכלול נתיב ({value})"))
            }
            FilterRule::Extension { value } if patterns(value).is_empty() => {
                return Err("כלל סינון: יש להזין סיומת או שם קובץ (למשל tmp)".into())
            }
            FilterRule::Extension { value } if value.contains('\\') => {
                return Err(format!("כלל סינון: שם קובץ לא יכול לכלול נתיב ({value})"))
            }
            FilterRule::Regex { value, .. } if value.trim().is_empty() => {
                return Err("כלל סינון: יש להזין ביטוי רגולרי".into())
            }
            FilterRule::Regex { value, .. } => {
                if let Err(e) = compile_regex(value.trim()) {
                    return Err(format!("כלל סינון: הביטוי הרגולרי \"{value}\" לא תקין ({e})"));
                }
            }
            FilterRule::Pattern { value } if value.trim().is_empty() => return Err("כלל סינון: יש להזין תבנית שם קובץ".into()),
            FilterRule::Pattern { value } if value.contains('\\') => {
                return Err(format!(
                    "כלל סינון: תבנית שם קובץ לא יכולה לכלול נתיב ({value}). לתיקיות יש כלל \"תיקייה\""
                ))
            }
            FilterRule::Folder { value } if value.trim().is_empty() => {
                return Err("כלל סינון: יש להזין שם או נתיב של תיקייה".into())
            }
            FilterRule::LargerThan { mb: 0 } => return Err("כלל סינון: גודל הקובץ חייב להיות לפחות 1MB".into()),
            FilterRule::OlderThan { days: 0 } => return Err("כלל סינון: מספר הימים חייב להיות לפחות 1".into()),
            _ => {}
        }
    }
    Ok(())
}

pub fn compile<'a>(rules: impl IntoIterator<Item = &'a FilterRule>) -> Filters {
    let mut f = Filters::default();
    for r in rules {
        match r {
            FilterRule::Include { value } => f.include_files.extend(patterns(value)),
            FilterRule::Extension { value } => f.exclude_files.extend(patterns(value)),
            FilterRule::Pattern { value } => f.exclude_files.push(value.trim().to_string()),
            FilterRule::Regex { value, include: true } => f.include_regex.push(value.trim().to_string()),
            FilterRule::Regex { value, include: false } => f.exclude_regex.push(value.trim().to_string()),
            FilterRule::Folder { value } => f.exclude_dirs.push(value.trim().to_string()),
            FilterRule::LargerThan { mb } => {
                let bytes = mb * 1024 * 1024;
                f.max_size = Some(f.max_size.map_or(bytes, |m| m.min(bytes)));
            }
            FilterRule::OlderThan { days } => f.max_age_days = Some(f.max_age_days.map_or(*days, |d| d.min(*days))),
            FilterRule::Hidden => f.exclude_hidden = true,
            FilterRule::System => f.exclude_system = true,
        }
    }
    f.include_files.dedup();
    f.exclude_files.dedup();
    f.exclude_dirs.dedup();
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_rules() {
        let f = compile(&[
            FilterRule::Extension {
                value: "tmp, .log;*.bak".into(),
            },
            FilterRule::Pattern { value: "~$*".into() },
            FilterRule::Folder {
                value: "node_modules".into(),
            },
            FilterRule::LargerThan { mb: 100 },
            FilterRule::LargerThan { mb: 10 },
            FilterRule::Hidden,
        ]);
        assert_eq!(f.exclude_files, ["*.tmp", "*.log", "*.bak", "~$*"]);
        assert_eq!(f.exclude_dirs, ["node_modules"]);
        assert_eq!(f.max_size, Some(10 * 1024 * 1024));
        assert!(f.exclude_hidden && !f.exclude_system);
    }

    #[test]
    fn include_accepts_plain_extensions() {
        assert_eq!(
            patterns("lrcat, .docx; *.xlsx, notes.txt, ~$*"),
            ["*.lrcat", "*.docx", "*.xlsx", "notes.txt", "~$*"]
        );
    }

    #[test]
    fn regex_rules() {
        let rules = [
            FilterRule::Regex {
                value: r"^\d+\.jpg$".into(),
                include: true,
            },
            FilterRule::Regex {
                value: "copy".into(),
                include: false,
            },
        ];
        assert!(validate(&rules).is_ok());
        let f = compile(&rules);
        assert_eq!((f.include_regex.len(), f.exclude_regex.len()), (1, 1));
        assert!(validate(&[FilterRule::Regex {
            value: "(".into(),
            include: false
        }])
        .is_err());
        assert!(validate(&[FilterRule::Regex {
            value: " ".into(),
            include: false
        }])
        .is_err());
    }

    #[test]
    fn rejects_bad_rules() {
        assert!(validate(&[FilterRule::Extension { value: " , ".into() }]).is_err());
        assert!(validate(&[FilterRule::Pattern {
            value: "a\\b.txt".into()
        }])
        .is_err());
        assert!(validate(&[FilterRule::OlderThan { days: 0 }]).is_err());
        assert!(validate(&[FilterRule::Extension { value: "tmp".into() }]).is_ok());
    }
}
