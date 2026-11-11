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
  | { kind: "includeName"; value: string }
  | { kind: "extension"; value: string }
  | { kind: "pattern"; value: string }
  /** `folders`: matches folder names (always skipped, with their contents) instead of file names. */
  | { kind: "regex"; value: string; include: boolean; folders: boolean }
  | { kind: "folder"; value: string }
  /** One file by its full path (usually added from a run's errors). */
  | { kind: "file"; value: string }
  | { kind: "largerThan"; mb: number }
  | { kind: "olderThan"; days: number }
  | { kind: "hidden" }
  | { kind: "system" };

export type FilterKind = FilterRule["kind"];

/** What reading a filter list file gave: the valid rules, and how many entries were left out. */
export interface FilterImport {
  rules: FilterRule[];
  skipped: number;
}

/** Live reading of a regex filter: why it is invalid, what it says piece by piece, and a try-out on a sample name. */
export interface RegexCheck {
  error: string | null;
  parts: { token: string; meaning: string }[];
  note: string | null;
  /** null = no sample name given. */
  matched: boolean | null;
  matchedText: string | null;
}

/** A path to run through all the filter rules together (the filter check under the rule list). */
export interface FilterProbe {
  /** Full path, or relative to the task's source. Ending with "\\" = a folder. */
  path: string;
  /** Used only when the path isn't on disk. */
  sizeMb: number | null;
  ageDays: number | null;
  hidden: boolean;
  system: boolean;
}

export type RuleEffect = "none" | "skips" | "keeps" | "unknown" | "invalid";

export interface RuleResult {
  effect: RuleEffect;
  detail: string | null;
}

export interface FilterCheck {
  /** null = nothing to check (see notes). */
  kept: boolean | null;
  folder: boolean;
  /** The checked path in full (a relative one under the task's first source); null with no source to read it under. */
  fullPath: string | null;
  /** The same path as a folder, for a name that isn't on disk and wasn't typed with a trailing `\`. */
  asFolder: FilterCheck | null;
  reason: string | null;
  /** The real file the check used, when the path exists. */
  onDisk: { size: number; modified: string | null; hidden: boolean; system: boolean } | null;
  notes: string[];
  /** One per rule, in order. */
  rules: RuleResult[];
  globalRules: RuleResult[];
}

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
  /** The task list (group) it belongs to; "" = the general list. */
  group: string;
  /** Imported from Cobian; waiting for the user to take over (or decline) Cobian's backups. Nothing runs meanwhile. */
  cobianPending: boolean;
  /** Continue from Cobian's backups: they are renamed right before the task's next run (not before). */
  cobianTakeover: boolean;
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
  /** The last run was paused and its folder is still there: the next run continues in it. */
  resumable?: boolean;
  /** The newest backup that was written (a failed or stopped run doesn't replace it). */
  lastBackup?: LastBackup | null;
  /** The next automatic run (scheduled, catch-up, drive connected) is a full backup, once. */
  fullNext?: boolean;
}

/** Size of a task's newest backup (all its sources). Full if any source made a full one. */
export interface LastBackup {
  at: string;
  bytes: number;
  mode: BackupMode;
}

/** Free and total bytes of a drive. */
export interface DiskSpace {
  free: number;
  total: number;
}

/** What a task's next full backup needs on the destination (null = no full backup expected). */
export interface FullSpace {
  /** About the size of the previous full backup(s). */
  needed: number;
  /** What delete-before removes first (0 without it); counts as free space. */
  freed: number;
}

export interface Progress {
  runId: string;
  taskId: string;
  taskName: string;
  mode: BackupMode;
  phase: "scanning" | "deleting" | "pruning" | "copying";
  startedAt: string;
  sourceIndex: number;
  sourceCount: number;
  sourcePath: string;
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
  /** Where the totals come from: counters only | estimated from the last backup | counted. */
  totals: "unknown" | "estimate" | "exact";
  /** A scan beside the copy is still counting (it can be skipped). */
  totalsScan: boolean;
  /** When copying began for the current source (not the scan before it). */
  copyStartedAt: string | null;
  /** Bytes per second of the task's last real copy. */
  expectedRate: number | null;
  /** Bytes the sources after the current one are expected to copy, from their last runs (null = unknown, 0 = last source). */
  laterBytes: number | null;
  /** Bytes the sources before the current one copied in this run. */
  earlierBytes: number;
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
  /** "auto" also installs with the window open after this many idle minutes (0 = only when closed/minimized). */
  updateIdleMinutes: number;
  /** Walk the whole tree once before a full copy, just for an exact percentage (slow on big drives). */
  exactProgress: boolean;
  backgroundTotals: boolean;
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
  /** Show the update dialog: the "new version" notification was clicked (or the window opened after it). */
  prompt: boolean;
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
  /** Backup folders that should have been deleted but weren't (shown by DeleteProblemDialog). */
  deleteProblems: DeleteProblem[];
  /** Shown until dismissed (e.g. the task list was recovered from a backup). */
  notice: Notice | null;
  update: UpdateStatus;
  /** Per task id: how many of its folders a Cobian takeover renamed (their names can be put back). */
  cobianRenamed: Record<string, number>;
}

/** A backup folder that should have been deleted but wasn't (model.rs `DeleteProblem`). */
export interface DeleteProblem {
  taskId: string;
  taskName: string;
  path: string;
  /** access = needs admin, inUse = a program has a file open. */
  reason: "access" | "inUse" | "other";
  /** Windows's own message. */
  message: string;
  at: string;
  /** The running backup waits for the answer before it copies (a delete-before failed). */
  blocking: boolean;
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
  /** What the file looked like when it failed (attributes, size, destination...); only in the copied error text. */
  debug?: string;
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
  /** How the source was copied (engine command line, volumes...); only in the copied error text. */
  debug?: string[];
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
  /** Size of the folder's files; null when not known (a partial folder). */
  bytes: number | null;
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

/** How a plain JSON config becomes tasks: all folders in one, one per `src_N` group, one per folder. */
export type ImportSplit = "one" | "group" | "source";

export interface ImportPreview {
  source: "backuper" | "simple" | "cobian";
  tasks: ImportedTask[];
  /** Plain config only: non-empty groups and distinct folders in the file. */
  simple: { groups: number; sources: number } | null;
  /** Tasks of an earlier import of a plain config, split differently - deleted when importing. */
  replaces: { id: string; name: string }[];
}

/** A Cobian Backup / Reflector task list found on this computer. */
export interface CobianList {
  path: string;
  /** File name without .lst ("MainList"). */
  name: string;
  /** The program folder it's in ("Cobian Reflector"). */
  product: string;
  modified: string | null;
  taskCount: number | null;
  error: string | null;
}

export interface CobianLists {
  lists: CobianList[];
  /** Where a file dialog for picking a list should open. */
  dir: string | null;
}

/** One Cobian backup folder and the name it gets here (takeover = rename only). */
export interface AdoptFolder {
  from: string;
  to: string;
  kind: BackupMode;
  createdAt: string;
}

export interface AdoptSource {
  source: string;
  folderName: string;
  /** Oldest first. */
  folders: AdoptFolder[];
  /** Something is wrong or can't be taken over. */
  warnings: string[];
  /** Old incrementals Cobian left without their full (normal). Not taken over; one note per task in the UI. */
  leftovers: Leftovers | null;
}

export interface Leftovers {
  count: number;
  first: string;
  last: string;
}

/** Which Cobian backup folders a task would take over, so it continues from Cobian's last backup. */
export interface AdoptPlan {
  taskId: string;
  destination: string;
  sources: AdoptSource[];
}

export interface AdoptResult {
  renamed: number;
  errors: string[];
}

/**
 * How Cobian named its backups - only for folders taken over by an older version, whose original
 * names weren't recorded. `date`: "ymd-hms" (Cobian's default) | "ymd-hm" | "yy-hms" | "yy-hm";
 * `kind`: "" (no type) | "word" (" Full") | "paren" (" (Full)").
 */
export interface CobianNaming {
  date: string;
  kind: string;
}

/** One taken-over folder and the Cobian name it gets back. */
export interface RestoreFolder {
  from: string;
  to: string;
  kind: BackupMode;
  /** Why it can't be renamed back; null = it can. */
  problem: string | null;
}

export interface RestorePlan {
  taskId: string;
  destination: string;
  folders: RestoreFolder[];
  /** No record of the renames: names rebuilt from `CobianNaming`. */
  legacy: boolean;
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
  group: "",
  cobianPending: false,
  cobianTakeover: false,
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
