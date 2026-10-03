//! User filter rules -> engine filters.

use crate::engine::Filters;
use crate::model::FilterRule;

/// "tmp, .log;*.bak" -> ["*.tmp", "*.log", "*.bak"]
fn extensions(value: &str) -> Vec<String> {
    value
        .split([',', ';', ' '])
        .map(|e| e.trim().trim_start_matches('*').trim_start_matches('.'))
        .filter(|e| !e.is_empty())
        .map(|e| format!("*.{e}"))
        .collect()
}

/// "*.lrcat, *.docx;notes.txt" -> ["*.lrcat", "*.docx", "notes.txt"] (names may contain spaces).
pub fn patterns(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
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
            FilterRule::Extension { value } if extensions(value).is_empty() => {
                return Err("כלל סינון: יש להזין סיומת (למשל tmp)".into())
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
            FilterRule::Extension { value } => f.exclude_files.extend(extensions(value)),
            FilterRule::Pattern { value } => f.exclude_files.push(value.trim().to_string()),
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
