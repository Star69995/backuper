//! JSON persistence under %APPDATA%\org.tovtech.backuper. Writes are atomic (tmp + rename).

use crate::model::{RunRecord, Settings, Task, TaskState};
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

impl Store {
    pub fn open(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("logs"));
        let mut tasks: Vec<Task> = load(&dir.join("tasks.json"));
        tasks.iter_mut().for_each(Task::migrate);
        Self {
            tasks,
            states: load(&dir.join("state.json")),
            settings: load(&dir.join("settings.json")),
            history: load(&dir.join("history.json")),
            dir,
        }
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.dir.join("logs")
    }

    pub fn save_tasks(&self) {
        save(&self.dir.join("tasks.json"), &self.tasks);
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
