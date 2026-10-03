import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { BackupMode, ImportPreview, Progress, RunRecord, Schedule, Settings, Snapshot, SourceBackups, Task, TaskSnapshot } from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  saveTasks: (tasks: Task[]) => invoke<Task[]>("save_tasks", { tasks }),
  deleteTasks: (ids: string[]) => invoke<void>("delete_tasks", { ids }),
  reorderTasks: (ids: string[]) => invoke<void>("reorder_tasks", { ids }),
  runTasks: (ids: string[], mode: BackupMode | null = null) => invoke<number>("run_tasks", { ids, mode }),
  /** Answers the drive-connected question: `run` get queued, all of `ids` stop waiting. */
  answerDrivePrompts: (ids: string[], run: string[]) => invoke<void>("answer_drive_prompts", { ids, run }),
  cancelTask: (id: string) => invoke<void>("cancel_task", { id }),
  history: () => invoke<RunRecord[]>("get_history"),
  clearHistory: () => invoke<void>("clear_history"),
  readLog: (runId: string, sourceIndex: number) => invoke<string>("read_log", { runId, sourceIndex }),
  listBackups: (taskId: string) => invoke<SourceBackups[]>("list_backups", { taskId }),
  deleteBackup: (taskId: string, folderName: string, name: string) => invoke<void>("delete_backup", { taskId, folderName, name }),
  folderSize: (path: string) => invoke<number>("folder_size", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  previewSchedule: (schedule: Schedule) => invoke<string[]>("preview_schedule", { schedule }),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  importTasks: (path: string) => invoke<ImportPreview>("import_tasks", { path }),
  /** All tasks when `ids` is null. Returns how many were saved. */
  exportTasks: (path: string, ids: string[] | null = null) => invoke<number>("export_tasks", { path, ids }),
  /** The automatic snapshots, or those in `dir` (a folder the user picked). */
  listTaskSnapshots: (dir: string | null = null) => invoke<TaskSnapshot[]>("list_task_snapshots", { dir }),
  dismissNotice: () => invoke<void>("dismiss_notice"),
  /** Shows a sample notification with this sound. */
  testSound: (sound: string) => invoke<void>("test_sound", { sound }),
  /** Checks for a newer version now; true = one is available. */
  checkUpdates: () => invoke<boolean>("check_updates"),
  /** Downloads and installs it (the app restarts), right away or once the backups are done. */
  installUpdate: () => invoke<void>("install_update"),
  saveFile: async (title: string, defaultPath: string, filterName: string, extensions: string[]) => {
    const r = await save({ title, defaultPath, filters: [{ name: filterName, extensions }] });
    return typeof r === "string" ? r : null;
  },
  pickFile: async (title: string, filterName: string, extensions: string[]) => {
    const r = await open({ multiple: false, title, filters: [{ name: filterName, extensions }] });
    return typeof r === "string" ? r : null;
  },
  pickFolder: async (title: string, defaultPath?: string) => {
    const r = await open({ directory: true, multiple: false, title, defaultPath: defaultPath || undefined });
    return typeof r === "string" ? r : null;
  },
  /** Windows folder dialog with multi-select (Ctrl/Shift+click). */
  pickFolders: async (title: string) => {
    const r = await open({ directory: true, multiple: true, title });
    return Array.isArray(r) ? r : typeof r === "string" ? [r] : [];
  },
  onSnapshotChanged: (cb: () => void) => listen("snapshot-changed", cb),
  onProgress: (cb: (p: Progress) => void) => listen<Progress>("progress", (e) => cb(e.payload)),
};

export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
