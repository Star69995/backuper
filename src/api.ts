import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  AdoptPlan,
  AdoptResult,
  BackupMode,
  CobianLists,
  CobianNaming,
  DiskSpace,
  FullSpace,
  ImportPreview,
  ImportSplit,
  Progress,
  RegexCheck,
  RestorePlan,
  FilterCheck,
  FilterImport,
  FilterProbe,
  FilterRule,
  RunRecord,
  Schedule,
  Settings,
  Snapshot,
  SourceBackups,
  Task,
  TaskSnapshot,
} from "./types";

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  saveTasks: (tasks: Task[]) => invoke<Task[]>("save_tasks", { tasks }),
  deleteTasks: (ids: string[]) => invoke<void>("delete_tasks", { ids }),
  reorderTasks: (ids: string[]) => invoke<void>("reorder_tasks", { ids }),
  /** `fresh`: start a new backup folder even if a paused one could be continued. */
  runTasks: (ids: string[], mode: BackupMode | null = null, fresh = false) => invoke<number>("run_tasks", { ids, mode, fresh }),
  /** Continues a task whose last run was stopped (false = already queued or running). */
  resumeTask: (id: string) => invoke<boolean>("resume_task", { id }),
  /** Answers the drive-connected question: `run` get queued, all of `ids` stop waiting. */
  answerDrivePrompts: (ids: string[], run: string[]) => invoke<void>("answer_drive_prompts", { ids, run }),
  /** Pauses a running task (its folder is kept, the next run continues in it); `discard` stops it for good and deletes the half-done folder. */
  cancelTask: (id: string, discard = false) => invoke<void>("cancel_task", { id, discard }),
  /** Takes a paused backup off the paused list; `del` also deletes its partial folders. Rejects with a message if it can't. */
  dismissPaused: (id: string, del: boolean) => invoke<void>("dismiss_paused", { id, delete: del }),
  /** Gives up on the scan that works out how much a running full backup has to copy. */
  skipTotalsScan: () => invoke<void>("skip_totals_scan"),
  history: () => invoke<RunRecord[]>("get_history"),
  clearHistory: () => invoke<void>("clear_history"),
  readLog: (runId: string, sourceIndex: number) => invoke<string>("read_log", { runId, sourceIndex }),
  listBackups: (taskId: string) => invoke<SourceBackups[]>("list_backups", { taskId }),
  deleteBackup: (taskId: string, folderName: string, name: string) => invoke<void>("delete_backup", { taskId, folderName, name }),
  folderSize: (path: string) => invoke<number>("folder_size", { path }),
  /** Free space on the drive of each path (null = drive not connected), same order. */
  diskSpace: (paths: string[]) => invoke<(DiskSpace | null)[]>("disk_space", { paths }),
  fullSpace: (taskIds: string[]) => invoke<(FullSpace | null)[]>("full_space", { taskIds }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  previewSchedule: (schedule: Schedule) => invoke<string[]>("preview_schedule", { schedule }),
  /** Validates a regex filter, explains it, and tries it on `sample` (a file or folder name). */
  checkRegex: (pattern: string, sample: string | null) => invoke<RegexCheck>("check_regex", { pattern, sample }),
  /** What all the rules together (+ the global ones) do with one path, and each rule's part in it. */
  checkFilters: (rules: FilterRule[], globalRules: FilterRule[], sources: string[], probe: FilterProbe) =>
    invoke<FilterCheck>("check_filters", { rules, globalRules, sources, probe }),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  importTasks: (path: string, split?: ImportSplit) => invoke<ImportPreview>("import_tasks", { path, split }),
  /** All tasks when `ids` is null. Returns how many were saved. */
  exportTasks: (path: string, ids: string[] | null = null) => invoke<number>("export_tasks", { path, ids }),
  /** The rules of a filter editor as a file / the valid rules of such a file. */
  exportFilters: (path: string, rules: FilterRule[]) => invoke<void>("export_filters", { path, rules }),
  importFilters: (path: string) => invoke<FilterImport>("import_filters", { path }),
  /** The automatic snapshots, or those in `dir` (a folder the user picked). */
  listTaskSnapshots: (dir: string | null = null) => invoke<TaskSnapshot[]>("list_task_snapshots", { dir }),
  /** Cobian Backup / Reflector task lists on this computer, and the folder to open a file dialog in. */
  findCobianLists: () => invoke<CobianLists>("find_cobian_lists"),
  /** Which Cobian backup folders these tasks (saved or still in an import preview) would take over. */
  planCobianTakeover: (tasks: Task[]) => invoke<AdoptPlan[]>("plan_cobian_takeover", { tasks }),
  /**
   * "Continue from Cobian's backups" for saved tasks. Nothing is renamed now: the folders get this
   * app's names right before each task's next run.
   */
  takeOverCobian: (ids: string[]) => invoke<void>("take_over_cobian", { ids }),
  /** "Don't continue from Cobian's backups" (also cancels a takeover that hasn't run yet): they may run now. */
  declineCobianTakeover: (ids: string[]) => invoke<void>("decline_cobian_takeover", { ids }),
  /** Which of the tasks' taken-over folders would get Cobian's names back (one plan per task). */
  planCobianRestore: (ids: string[], naming: CobianNaming) => invoke<RestorePlan[]>("plan_cobian_restore", { ids, naming }),
  /** Gives them Cobian's names back and switches off the tasks that got some back. */
  restoreCobianNames: (ids: string[], naming: CobianNaming) => invoke<AdoptResult>("restore_cobian_names", { ids, naming }),
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
  pickFile: async (title: string, filterName: string, extensions: string[], defaultPath?: string) => {
    const r = await open({
      multiple: false,
      title,
      defaultPath: defaultPath || undefined,
      filters: [{ name: filterName, extensions }],
    });
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
  onShowUpdate: (cb: () => void) => listen("show-update", cb),
  onSnapshotChanged: (cb: () => void) => listen("snapshot-changed", cb),
  onProgress: (cb: (p: Progress) => void) => listen<Progress>("progress", (e) => cb(e.payload)),
};

export const errorText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
