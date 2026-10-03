//! Detecting drives that get connected (external disks, USB sticks), for tasks that start
//! (or ask to start) when their drives become available.

use crate::model::{DriveAction, Task};
use std::collections::HashMap;
use std::path::Path;

/// The drive roots ("E:\") a task needs: its destination and sources. UNC paths aren't watched.
pub fn task_drives(t: &Task) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in std::iter::once(t.destination.as_str()).chain(t.sources.iter().map(|s| s.path.as_str())) {
        let b = p.trim().as_bytes();
        if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            let root = format!("{}:\\", (b[0] as char).to_ascii_uppercase());
            if !out.contains(&root) {
                out.push(root);
            }
        }
    }
    out
}

pub fn validate(t: &Task) -> Result<(), String> {
    if t.on_drive_connect != DriveAction::Off && task_drives(t).is_empty() {
        return Err("הפעלה בחיבור כונן זמינה רק לנתיבים עם אות כונן (למשל E:\\)".into());
    }
    Ok(())
}

/// A drive with no media (empty card reader) or a disconnected disk has no readable root.
fn is_present(root: &str) -> bool {
    Path::new(root).is_dir()
}

/// True when the task's drives were not all there before, and are all there now.
fn became_ready(drives: &[String], before: &HashMap<String, bool>, now: &HashMap<String, bool>) -> bool {
    let all_now = drives.iter().all(|d| now.get(d).copied().unwrap_or(false));
    let all_before = drives.iter().all(|d| before.get(d).copied().unwrap_or(false));
    all_now && !all_before
}

/// Remembers which drives were present at the last poll.
#[derive(Default)]
pub struct DriveWatch {
    present: HashMap<String, bool>,
}

impl DriveWatch {
    /// Checks the drives of the watched tasks; returns the ones whose drives just became available.
    /// A drive seen for the first time counts as "already there", so nothing fires at startup
    /// or right after a task is saved.
    pub fn poll<'a>(&mut self, tasks: &'a [Task]) -> Vec<&'a Task> {
        let watched: Vec<(&Task, Vec<String>)> = tasks
            .iter()
            .filter(|t| t.on_drive_connect != DriveAction::Off)
            .map(|t| (t, task_drives(t)))
            .collect();
        let mut now: HashMap<String, bool> = HashMap::new();
        for (_, drives) in &watched {
            for d in drives {
                if !now.contains_key(d) {
                    now.insert(d.clone(), is_present(d));
                }
            }
        }
        for (d, p) in &now {
            self.present.entry(d.clone()).or_insert(*p);
        }
        let ready = watched
            .into_iter()
            .filter(|(_, drives)| became_ready(drives, &self.present, &now))
            .map(|(t, _)| t)
            .collect();
        self.present = now;
        ready
    }

    /// Whether all of the task's drives were present at the last poll.
    pub fn all_present(&self, t: &Task) -> bool {
        task_drives(t).iter().all(|d| self.present.get(d).copied().unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn task(dest: &str, sources: &[&str]) -> Task {
        Task {
            destination: dest.into(),
            sources: sources
                .iter()
                .map(|p| Source {
                    path: p.to_string(),
                    folder_name: "x".into(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn drives_of_a_task() {
        let t = task("e:\\Backups", &["C:\\Users\\Me", "c:\\Data", "\\\\nas\\share"]);
        assert_eq!(task_drives(&t), vec!["E:\\", "C:\\"]);
        assert!(task_drives(&task("\\\\nas\\b", &["\\\\nas\\a"])).is_empty());
    }

    #[test]
    fn ready_only_on_transition_to_all_present() {
        let drives = vec!["C:\\".to_string(), "E:\\".to_string()];
        let m = |c: bool, e: bool| HashMap::from([("C:\\".to_string(), c), ("E:\\".to_string(), e)]);
        assert!(became_ready(&drives, &m(true, false), &m(true, true)));
        assert!(!became_ready(&drives, &m(true, true), &m(true, true)));
        assert!(!became_ready(&drives, &m(true, false), &m(true, false)));
        assert!(!became_ready(&drives, &m(false, false), &m(false, true)));
    }

    #[test]
    fn first_poll_never_fires() {
        let mut t = task("C:\\Backups", &["C:\\Windows"]);
        t.on_drive_connect = DriveAction::Run;
        let tasks = vec![t];
        let mut w = DriveWatch::default();
        assert!(w.poll(&tasks).is_empty());
        assert!(w.all_present(&tasks[0]));
        // Simulate the drive having been gone at the previous poll.
        w.present.insert("C:\\".into(), false);
        assert_eq!(w.poll(&tasks).len(), 1);
        assert!(w.poll(&tasks).is_empty());
    }
}
