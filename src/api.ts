import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { BackupFolder, BackupMode, Progress, RunRecord, Schedule, Settings, Snapshot, Task } from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  saveTasks: (tasks: Task[]) => invoke<Task[]>("save_tasks", { tasks }),
  deleteTasks: (ids: string[]) => invoke<void>("delete_tasks", { ids }),
  reorderTasks: (ids: string[]) => invoke<void>("reorder_tasks", { ids }),
  runTasks: (ids: string[], mode: BackupMode | null = null) => invoke<number>("run_tasks", { ids, mode }),
  cancelTask: (id: string) => invoke<void>("cancel_task", { id }),
  history: () => invoke<RunRecord[]>("get_history"),
  clearHistory: () => invoke<void>("clear_history"),
  readLog: (runId: string) => invoke<string>("read_log", { runId }),
  listBackups: (taskId: string) => invoke<BackupFolder[]>("list_backups", { taskId }),
  deleteBackup: (taskId: string, name: string) => invoke<void>("delete_backup", { taskId, name }),
  folderSize: (path: string) => invoke<number>("folder_size", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  previewSchedule: (schedule: Schedule) => invoke<string[]>("preview_schedule", { schedule }),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  pickFolder: async (title: string, defaultPath?: string) => {
    const r = await open({ directory: true, multiple: false, title, defaultPath: defaultPath || undefined });
    return typeof r === "string" ? r : null;
  },
  onSnapshotChanged: (cb: () => void) => listen("snapshot-changed", cb),
  onProgress: (cb: (p: Progress) => void) => listen<Progress>("progress", (e) => cb(e.payload)),
};

export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
