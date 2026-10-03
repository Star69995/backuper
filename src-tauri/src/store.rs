//! JSON persistence under %APPDATA%\org.tovtech.backuper. Writes are atomic (tmp + rename).

use crate::model::{Notice, RunRecord, Settings, Task, TaskState};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_HISTORY: usize = 2000;

fn remove_logs(r: &RunRecord) {
    for log in r.sources.iter().filter_map(|s| s.log_file.as_ref()) {
        let _ = fs::remove_file(log);
    }
}

pub struct Store {
    dir: PathBuf,
    pub tasks: Vec<Task>,
    pub states: HashMap<String, TaskState>,
    pub settings: Settings,
    pub history: Vec<RunRecord>,
    /// Shown in the UI until dismissed (not persisted).
    pub notice: Option<Notice>,
}

fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(path: &Path, value: &T) {
    let tmp = path.with_extension("tmp");
    if let Ok(json) = serde_json::to_string_pretty(value) {
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl Store {
    pub fn open(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("logs"));
        let mut store = Self {
            tasks: Vec::new(),
            states: load(&dir.join("state.json")),
            settings: load(&dir.join("settings.json")),
            history: load(&dir.join("history.json")),
            notice: None,
            dir,
        };
        store.load_tasks();
        store.tasks.iter_mut().for_each(Task::migrate);
        // The list as it was before this version kept snapshots.
        if !store.tasks.is_empty() {
            let _ = crate::tasklist::snapshot(&store.snapshots_dir(), &store.tasks);
        }
        store
    }

    /// Reads tasks.json. When it's damaged (or missing while snapshots exist), it's moved
    /// aside and the newest snapshot is loaded instead, with a notice for the user.
    fn load_tasks(&mut self) {
        let path = self.dir.join("tasks.json");
        let mut missing = false;
        let problem = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Vec<Task>>(&bytes) {
                Ok(tasks) => {
                    self.tasks = tasks;
                    return;
                }
                Err(_) => {
                    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
                    let aside = self.dir.join(format!("tasks - פגום {stamp}.json"));
                    match fs::rename(&path, &aside) {
                        Ok(()) => format!("קובץ רשימת המשימות היה פגום. הוא הועבר הצידה בשם \"{}\".", file_name(&aside)),
                        Err(_) => "קובץ רשימת המשימות היה פגום.".to_string(),
                    }
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                missing = true;
                "קובץ רשימת המשימות לא נמצא.".to_string()
            }
            Err(e) => format!("לא ניתן לקרוא את קובץ רשימת המשימות ({e})."),
        };

        let copy_dir = self.settings.task_list_copy_dir.trim();
        let mut dirs = vec![self.snapshots_dir()];
        if !copy_dir.is_empty() {
            dirs.push(Path::new(copy_dir).join(crate::tasklist::COPY_DIR_NAME));
        }
        let found = dirs
            .iter()
            .filter_map(|d| crate::tasklist::latest(d))
            .max_by_key(|(saved_at, _)| *saved_at);
        let Some((saved_at, tasks)) = found else {
            // Missing with nothing to recover from = a first run: start empty quietly.
            if !missing {
                self.notice = Some(Notice {
                    title: "רשימת המשימות לא נטענה".into(),
                    message: format!("{problem} לא נמצא גיבוי שלה, ולכן הרשימה ריקה. אפשר לשחזר מגיבוי בתיקייה אחרת בהגדרות."),
                    path: Some(self.dir.to_string_lossy().into_owned()),
                });
            }
            return;
        };
        let count = if tasks.len() == 1 {
            "משימה אחת".to_string()
        } else {
            format!("{} משימות", tasks.len())
        };
        self.notice = Some(Notice {
            title: "רשימת המשימות שוחזרה מגיבוי".into(),
            message: format!(
                "{problem} נטען הגיבוי האחרון שלה מ-{} ({count}). שינויים שנעשו אחרי הגיבוי הזה לא נשמרו - כדאי לבדוק את המשימות.",
                saved_at.format("%d/%m/%Y %H:%M")
            ),
            path: Some(self.dir.to_string_lossy().into_owned()),
        });
        self.tasks = tasks;
        save(&path, &self.tasks);
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.dir.join("logs")
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.dir.join("task-list-backups")
    }

    /// Saves the tasks, a dated snapshot, and one in the user's copy folder (if set).
    pub fn save_tasks(&self) {
        save(&self.dir.join("tasks.json"), &self.tasks);
        let _ = crate::tasklist::snapshot(&self.snapshots_dir(), &self.tasks);
        let _ = self.write_task_list_copy();
    }

    /// A snapshot in `settings.task_list_copy_dir` (no-op when not set).
    pub fn write_task_list_copy(&self) -> Result<(), String> {
        let dir = self.settings.task_list_copy_dir.trim();
        if dir.is_empty() {
            return Ok(());
        }
        if !Path::new(dir).is_dir() {
            return Err(format!("התיקייה לא נמצאה: {dir}"));
        }
        crate::tasklist::snapshot(&Path::new(dir).join(crate::tasklist::COPY_DIR_NAME), &self.tasks)
            .map_err(|e| format!("לא ניתן לשמור את רשימת המשימות ב-{dir}: {e}"))?;
        // The single copy older versions kept there is now stale (its content is in the snapshots).
        let _ = fs::remove_file(Path::new(dir).join(crate::tasklist::OLD_COPY_FILE_NAME));
        Ok(())
    }
    pub fn save_states(&self) {
        save(&self.dir.join("state.json"), &self.states);
    }
    pub fn save_settings(&self) {
        save(&self.dir.join("settings.json"), &self.settings);
    }

    pub fn add_history(&mut self, rec: RunRecord) {
        self.history.insert(0, rec);
        if self.history.len() > MAX_HISTORY {
            for old in self.history.drain(MAX_HISTORY..) {
                remove_logs(&old);
            }
        }
        self.save_history();
    }

    pub fn save_history(&self) {
        save(&self.dir.join("history.json"), &self.history);
    }

    pub fn clear_history(&mut self) {
        for r in self.history.drain(..) {
            remove_logs(&r);
        }
        self.save_history();
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str) -> Task {
        Task {
            id: name.into(),
            name: name.into(),
            ..Task::default()
        }
    }

    #[test]
    fn damaged_task_list_is_moved_aside_and_recovered() {
        let dir = std::env::temp_dir().join(format!("backuper-store-{}", uuid::Uuid::new_v4()));
        let mut store = Store::open(dir.clone());
        assert!(store.notice.is_none(), "a first run is quiet");
        store.tasks = vec![task("a"), task("b")];
        store.save_tasks();

        fs::write(dir.join("tasks.json"), "{ broken").unwrap();
        let store = Store::open(dir.clone());
        assert_eq!(store.tasks, [task("a"), task("b")]);
        assert!(store.notice.is_some());
        let aside = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("פגום"))
            .count();
        assert_eq!(aside, 1, "the damaged file is kept");
        assert_eq!(
            load::<Vec<Task>>(&dir.join("tasks.json")),
            [task("a"), task("b")],
            "the recovered list is saved"
        );

        // A list the user emptied on purpose stays empty.
        let mut store = Store::open(dir.clone());
        assert!(store.notice.is_none());
        store.tasks.clear();
        store.save_tasks();
        assert!(Store::open(dir.clone()).tasks.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
