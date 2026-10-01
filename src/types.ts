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

export interface Task {
  id: string;
  name: string;
  source: string;
  destination: string;
  folderName: string;
  mode: BackupMode;
  schedule: Schedule;
  enabled: boolean;
  keepCount: number;
  reusePrevious: boolean;
  fullEveryDays: number;
  copyEmptyDirs: boolean;
  catchUp: boolean;
  filters: FilterRule[];
  useGlobalFilters: boolean;
}

export type RunStatus = "success" | "warning" | "failed" | "cancelled";
export type Trigger = "manual" | "scheduled" | "catchUp";

export interface TaskState {
  nextRun: string | null;
  lastRunAt: string | null;
  lastStatus: RunStatus | null;
  lastMessage: string | null;
}

export interface Progress {
  runId: string;
  taskId: string;
  taskName: string;
  mode: BackupMode;
  phase: "scanning" | "copying" | "finishing";
  startedAt: string;
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
  firstRunDone: boolean;
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
  targetFolder: string | null;
  filesCopied: number;
  bytesCopied: number;
  filesDeleted: number;
  filesFailed: number;
  errors: string[];
  exitCode: number | null;
  logFile: string | null;
}

export interface BackupFolder {
  name: string;
  path: string;
  createdAt: string;
  partial: boolean;
}

export const newTask = (): Task => ({
  id: "",
  name: "",
  source: "",
  destination: "",
  folderName: "",
  mode: "incremental",
  schedule: { kind: "daily", time: "03:00" },
  enabled: true,
  keepCount: 1,
  reusePrevious: false,
  fullEveryDays: 30,
  copyEmptyDirs: false,
  catchUp: true,
  filters: [],
  useGlobalFilters: true,
});
