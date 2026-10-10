// Mirrors src-tauri/src/model.rs (camelCase JSON).

export type BackupMode = "full" | "incremental";

export type Schedule =
  | { kind: "manual" }
  | { kind: "once"; at: string }
  | { kind: "daily"; time: string }
  | { kind: "weekly"; days: number[]; time: string }
  | { kind: "monthly"; day: number; time: string }
  | { kind: "interval"; minutes: number };

export type ScheduleKind = Schedule["kind"];

export type FilterRule =
  | { kind: "include"; value: string }
  | { kind: "extension"; value: string }
  | { kind: "pattern"; value: string }
  | { kind: "regex"; value: string; include: boolean }
  | { kind: "folder"; value: string }
  | { kind: "largerThan"; mb: number }
  | { kind: "olderThan"; days: number }
  | { kind: "hidden" }
  | { kind: "system" };

export type FilterKind = FilterRule["kind"];

/** What a task does when its drives (destination / sources) get connected. */
export type DriveAction = "off" | "run" | "ask";

export interface Source {
  path: string;
  /** Prefix of this source's dated folders. */
  folderName: string;
}

/** count = newest keepCount fulls, days = enough to restore the last keepDays days, all = never delete. */
export type KeepMode = "count" | "days" | "all";

export interface Task {
  id: string;
  name: string;
  sources: Source[];
  destination: string;
  /** Backup type of the main schedule. */
  mode: BackupMode;
  schedule: Schedule;
  /** Combined mode (mode = incremental): full backups run on this schedule. */
  fullSchedule: Schedule | null;
  enabled: boolean;
  keepMode: KeepMode;
  keepCount: number;
  keepDays: number;
  deleteBefore: boolean;
  reusePrevious: boolean;
  deleteEmptyIncrementals: boolean;
  copyEmptyDirs: boolean;
  catchUp: boolean;
  onDriveConnect: DriveAction;
  filters: FilterRule[];
  useGlobalFilters: boolean;
}

export type RunStatus = "success" | "warning" | "failed" | "cancelled";
export type Trigger = "manual" | "scheduled" | "catchUp" | "driveConnected";

export interface TaskState {
  nextRun: string | null;
  nextFullRun: string | null;
  lastRunAt: string | null;
  lastStatus: RunStatus | null;
  lastMessage: string | null;
}

export interface Progress {
  runId: string;
  taskId: string;
  taskName: string;
  mode: BackupMode;
  phase: "scanning" | "deleting" | "copying";
  startedAt: string;
  sourceIndex: number;
  sourceCount: number;
  sourcePath: string;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  currentFile: string;
}

export interface Job {
  taskId: string;
  mode: BackupMode | null;
  trigger: Trigger;
}

export interface Settings {
  theme: "system" | "light" | "dark";
  notifySuccess: boolean;
  notifyFailure: boolean;
  /** Windows toast sound name ("Default", "IM", "Mail", "Reminder", "SMS"). Empty = silent. */
  soundSuccess: string;
  /** Used for failures, warnings and notices. */
  soundFailure: string;
  schedulerPaused: boolean;
  closeToTray: boolean;
  startWithWindows: boolean;
  /** Folder that also gets the task list snapshots (in a hidden subfolder). Empty = off. */
  taskListCopyDir: string;
  globalFilters: FilterRule[];
  /** auto = download and install by itself (from the tray, between backups); notify = only say so. */
  updateMode: "auto" | "notify" | "off";
}

/** Where the app's self-update stands. */
export interface UpdateStatus {
  currentVersion: string;
  state: "idle" | "checking" | "upToDate" | "available" | "downloading" | "ready" | "error";
  /** The newer version, once one was found. */
  version: string | null;
  notes: string | null;
  downloaded: number;
  total: number | null;
  checkedAt: string | null;
  error: string | null;
  /** The user asked to install; it waits for the running/queued backups to finish. */
  installWaiting: boolean;
}

export interface Snapshot {
  tasks: Task[];
  states: Record<string, TaskState>;
  current: Progress | null;
  queue: Job[];
  settings: Settings;
  autostart: boolean;
  /** Ids of tasks whose drive was connected, waiting for "back up now?". */
  drivePrompts: string[];
  /** Shown until dismissed (e.g. the task list was recovered from a backup). */
  notice: Notice | null;
  update: UpdateStatus;
}

export interface Notice {
  title: string;
  message: string;
  /** A file or folder the user may want to see. */
  path: string | null;
}

/** A file that couldn't be copied. `code` = Win32 error code (explained in lib/fileErrors.ts). */
export interface FileError {
  path: string;
  code: number | null;
  message: string;
}

export interface SourceRun {
  source: string;
  folderName: string;
  mode: BackupMode | null;
  status: RunStatus | null;
  message: string;
  targetFolder: string | null;
  filesCopied: number;
  bytesCopied: number;
  /** Size of the finished backup folder. */
  backupBytes: number;
  /** Space freed by deleting older backups. */
  freedBytes: number;
  filesDeleted: number;
  filesFailed: number;
  errors: FileError[];
  exitCode: number | null;
  logFile: string | null;
}

export interface RunRecord {
  id: string;
  taskId: string;
  taskName: string;
  trigger: Trigger;
  mode: BackupMode;
  startedAt: string;
  finishedAt: string;
  status: RunStatus;
  message: string;
  filesCopied: number;
  bytesCopied: number;
  /** Size of the finished backup folder. */
  backupBytes: number;
  /** Space freed by deleting older backups. */
  freedBytes: number;
  filesDeleted: number;
  filesFailed: number;
  sources: SourceRun[];
}

export interface BackupFolder {
  name: string;
  path: string;
  createdAt: string;
  kind: BackupMode;
  partial: boolean;
}

export interface SourceBackups {
  source: string;
  folderName: string;
  backups: BackupFolder[];
}

/** A task read from a task list file (export, snapshot or Cobian), before it's saved. */
export interface ImportedTask {
  task: Task;
  /** Settings that couldn't be carried over exactly. */
  warnings: string[];
  /** Why it can't be imported as is. */
  error: string | null;
  /** A task with this id exists (importing again updates it). */
  exists: boolean;
  /** It exists and is identical, so importing it changes nothing. */
  unchanged: boolean;
}

export interface ImportPreview {
  source: "backuper" | "simple" | "cobian";
  tasks: ImportedTask[];
}

/** An automatic copy of the task list, taken whenever it changed. */
export interface TaskSnapshot {
  path: string;
  savedAt: string;
  taskCount: number;
}

export const newTask = (): Task => ({
  id: "",
  name: "",
  sources: [],
  destination: "",
  mode: "incremental",
  schedule: { kind: "daily", time: "03:00" },
  fullSchedule: null,
  enabled: true,
  keepMode: "count",
  keepCount: 1,
  keepDays: 30,
  deleteBefore: false,
  reusePrevious: false,
  deleteEmptyIncrementals: true,
  copyEmptyDirs: false,
  catchUp: true,
  onDriveConnect: "off",
  filters: [],
  useGlobalFilters: true,
});
