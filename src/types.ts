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
  | { kind: "extension"; value: string }
  | { kind: "pattern"; value: string }
  | { kind: "folder"; value: string }
  | { kind: "largerThan"; mb: number }
  | { kind: "olderThan"; days: number }
  | { kind: "hidden" }
  | { kind: "system" };

export type FilterKind = FilterRule["kind"];

export interface Source {
  path: string;
  /** Prefix of this source's dated folders. */
  folderName: string;
}

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
  keepCount: number;
  deleteBefore: boolean;
  reusePrevious: boolean;
  deleteEmptyIncrementals: boolean;
  copyEmptyDirs: boolean;
  catchUp: boolean;
  filters: FilterRule[];
  useGlobalFilters: boolean;
}

export type RunStatus = "success" | "warning" | "failed" | "cancelled";
export type Trigger = "manual" | "scheduled" | "catchUp";

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
  schedulerPaused: boolean;
  closeToTray: boolean;
  startWithWindows: boolean;
  globalFilters: FilterRule[];
}

export interface Snapshot {
  tasks: Task[];
  states: Record<string, TaskState>;
  current: Progress | null;
  queue: Job[];
  settings: Settings;
  autostart: boolean;
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
  filesDeleted: number;
  filesFailed: number;
  errors: string[];
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

/** A task read from a Cobian task list, before it's saved. */
export interface ImportedTask {
  task: Task;
  /** Settings that couldn't be carried over exactly. */
  warnings: string[];
  /** Why it can't be imported as is. */
  error: string | null;
  /** A task with this id exists (importing again updates it). */
  exists: boolean;
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
  keepCount: 1,
  deleteBefore: false,
  reusePrevious: false,
  deleteEmptyIncrementals: true,
  copyEmptyDirs: false,
  catchUp: true,
  filters: [],
  useGlobalFilters: true,
});
