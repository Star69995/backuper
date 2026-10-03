// Dev-only: lets the UI run in a plain browser (`npm run dev`) with fake data,
// for visual checks and screenshots. Never loaded inside the real Tauri app.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { ImportedTask, ImportPreview, Notice, RunRecord, Schedule, Settings, Snapshot, SourceBackups, SourceRun, Task, TaskState, UpdateStatus } from "../types";
import { newTask } from "../types";

const iso = (minutesFromNow: number) => new Date(Date.now() + minutesFromNow * 60_000).toISOString();
const src = (path: string, folderName: string) => ({ path, folderName });

/** The next `n` times a schedule fires (or, with `back`, the previous ones), so mock times match the schedules shown. */
function occurrences(s: Schedule, n: number, back = false): string[] {
  if (s.kind === "interval") return Array.from({ length: n }, (_, i) => iso((back ? -1 : 1) * (i + 1) * s.minutes));
  if (!("time" in s)) return [];
  const [h, m] = s.time.split(":").map(Number);
  const out: string[] = [];
  for (let i = 0; out.length < n && i < 400; i++) {
    const d = new Date();
    d.setDate(d.getDate() + (back ? -i : i));
    d.setHours(h, m, 0, 0);
    const fires =
      s.kind === "daily" ||
      (s.kind === "weekly" && s.days.includes(d.getDay())) ||
      (s.kind === "monthly" && d.getDate() === s.day);
    if (fires && (back ? d.getTime() < Date.now() : d.getTime() > Date.now())) out.push(d.toISOString());
  }
  return out;
}
const next = (s: Schedule) => occurrences(s, 1)[0] ?? null;
const last = (s: Schedule) => occurrences(s, 1, true)[0] ?? null;

export function installDevMock() {
  const tasks: Task[] = [
    {
      ...newTask(),
      id: "1",
      name: "מסמכים ותמונות",
      sources: [
        src("C:\\Users\\User\\Documents", "Documents"),
        src("C:\\Users\\User\\Pictures", "Pictures"),
        src("C:\\Users\\User\\Desktop", "Desktop"),
      ],
      destination: "E:\\Backups",
      schedule: { kind: "daily", time: "03:00" },
      fullSchedule: { kind: "weekly", days: [5], time: "03:00" },
    },
    {
      ...newTask(),
      id: "2",
      name: "תמונות משפחה",
      sources: [src("D:\\Photos", "תמונות")],
      destination: "E:\\Backups",
      mode: "full",
      keepMode: "days",
      keepDays: 14,
      deleteBefore: true,
      schedule: { kind: "weekly", days: [5], time: "22:00" },
    },
    {
      ...newTask(),
      id: "3",
      name: "פרויקטים",
      sources: [src("C:\\code", "Projects")],
      destination: "F:\\Mirror",
      schedule: { kind: "interval", minutes: 120 },
      filters: [
        { kind: "folder", value: "node_modules" },
        { kind: "extension", value: "tmp, log" },
      ],
    },
    {
      ...newTask(),
      id: "4",
      name: "הגדרות תוכנות",
      sources: [src("C:\\Users\\User\\AppData\\Roaming", "AppData")],
      destination: "E:\\Backups",
      enabled: false,
      schedule: { kind: "monthly", day: 1, time: "12:00" },
    },
  ];
  const states: Record<string, TaskState> = {
    "1": {
      nextRun: next(tasks[0].schedule),
      nextFullRun: next(tasks[0].fullSchedule!),
      lastRunAt: last(tasks[0].schedule),
      lastStatus: "success",
      lastMessage: "3 תיקיות, 3 הצליחו. הועתקו 132 קבצים",
    },
    "2": {
      nextRun: next(tasks[1].schedule),
      nextFullRun: null,
      lastRunAt: last(tasks[1].schedule),
      lastStatus: "warning",
      lastMessage: "נמצאו פריטים לא תואמים",
    },
    "3": { nextRun: iso(47), nextFullRun: null, lastRunAt: iso(-73), lastStatus: "failed", lastMessage: "היעד לא זמין" },
    "4": { nextRun: null, nextFullRun: null, lastRunAt: null, lastStatus: null, lastMessage: null },
  };
  let settings: Settings = {
    theme: "system",
    notifySuccess: true,
    notifyFailure: true,
    soundSuccess: "Default",
    soundFailure: "Reminder",
    schedulerPaused: false,
    closeToTray: true,
    startWithWindows: true,
    taskListCopyDir: "",
    globalFilters: [{ kind: "pattern", value: "~$*" }],
    updateMode: "auto",
  };
  // Open with a newer version available (?update in the URL).
  const update: UpdateStatus = new URLSearchParams(location.search).has("update")
    ? {
        currentVersion: "0.3.4",
        state: "available",
        version: "0.4.0",
        notes: "- Self-update from GitHub releases\n- Small fixes",
        downloaded: 0,
        total: null,
        checkedAt: iso(-3),
        error: null,
        installWaiting: false,
      }
    : { currentVersion: "0.3.4", state: "upToDate", version: null, notes: null, downloaded: 0, total: null, checkedAt: iso(-90), error: null, installWaiting: false };
  const sourceRun = (folderName: string, source: string, i: number): SourceRun => ({
    source,
    folderName,
    mode: i === 0 ? "full" : "incremental",
    status: "success",
    message:
      i === 0
        ? "הועתקו 120 קבצים. אין גיבוי מלא קודם - מבוצע גיבוי מלא"
        : i === 1
          ? "הועתקו 12 קבצים חדשים או שהשתנו"
          : "אין שינויים מאז הגיבוי הקודם",
    targetFolder: `E:\\Backups\\${folderName} 2026-10-01 03-00 ${i === 0 ? "מלא" : "אינקרמנטלי"}`,
    filesCopied: i === 0 ? 120 : i === 1 ? 12 : 0,
    bytesCopied: i === 0 ? 480_000_000 : i === 1 ? 3_200_000 : 0,
    backupBytes: i === 0 ? 480_000_000 : i === 1 ? 3_200_000 : 0,
    freedBytes: i === 0 ? 455_000_000 : 0,
    filesDeleted: 0,
    filesFailed: 0,
    errors: [],
    exitCode: i === 0 ? 1 : null,
    logFile: "x",
  });
  const history: RunRecord[] = [1, 2, 3, 4].map((i) => {
    const t = tasks[(i - 1) % 3];
    const sources = t.sources.map((s, j) => sourceRun(s.folderName, s.path, j));
    if (i === 3) {
      Object.assign(sources[0], {
        status: "failed",
        message: "היעד לא זמין (F:\\Mirror): The system cannot find the path specified.",
        errors: [
          {
            path: "C:\\code\\locked.db",
            code: 32,
            message: "The process cannot access the file because it is being used by another process.",
          },
          ...["0bsw7oyftsd743.o", "0d37qz099npk2s.o", "0fkisol3qgcake.o"].map((f) => ({
            path: `C:\\code\\app\\target\\debug\\incremental\\s-hmvdvqo39o-working\\${f}`,
            code: 3,
            message: "The system cannot find the path specified. (os error 3)",
          })),
          { path: "C:\\code\\odd.bin", code: 1359, message: "An internal error occurred. (os error 1359)" },
        ],
        filesFailed: 5,
      });
    }
    // Matches the "warning" last status of task 2 in `states`.
    if (i === 2) {
      Object.assign(sources[0], {
        status: "warning",
        message: "הגיבוי הסתיים, אך נמצאו פריטים לא תואמים",
        errors: [],
        exitCode: 5,
      });
    }
    return {
      id: `r${i}`,
      taskId: t.id,
      taskName: t.name,
      trigger: i % 2 ? "scheduled" : "manual",
      mode: t.mode,
      startedAt: iso(-i * 300),
      finishedAt: iso(-i * 300 + 3),
      status: i === 3 ? "failed" : i === 2 ? "warning" : "success",
      message: sources.length > 1 ? `${sources.length} תיקיות, ${sources.length} הצליחו. הועתקו 132 קבצים` : sources[0].message,
      filesCopied: sources.reduce((n, s) => n + s.filesCopied, 0),
      bytesCopied: sources.reduce((n, s) => n + s.bytesCopied, 0),
      backupBytes: sources.reduce((n, s) => n + s.backupBytes, 0),
      freedBytes: sources.reduce((n, s) => n + s.freedBytes, 0),
      filesDeleted: 0,
      filesFailed: sources.reduce((n, s) => n + s.filesFailed, 0),
      sources,
    };
  });
  // The task's last 3 runs as dated folders, oldest one full (all full for a full-only task).
  const stamp = (at: string) => {
    const d = new Date(at);
    const p = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}-${p(d.getMinutes())}`;
  };
  const backupsOf = (t: Task): SourceBackups[] =>
    t.sources.map((s) => ({
      source: s.path,
      folderName: s.folderName,
      backups: occurrences(t.schedule, 3, true).map((createdAt, i) => {
        const kind = t.mode === "full" || i === 2 ? "full" : "incremental";
        const name = `${s.folderName} ${stamp(createdAt)} ${kind === "full" ? "מלא" : "אינקרמנטלי"}`;
        return { name, path: `${t.destination}\\${name}`, createdAt, kind, partial: false };
      }),
    }));
  // Open with the drive-connected question showing (?prompt in the URL).
  let drivePrompts = new URLSearchParams(location.search).has("prompt") ? ["1", "2"] : [];
  // Open with the "task list recovered" notice (?notice in the URL).
  let notice: Notice | null = new URLSearchParams(location.search).has("notice")
    ? {
        title: "רשימת המשימות שוחזרה מגיבוי",
        message:
          'קובץ רשימת המשימות היה פגום. הוא הועבר הצידה בשם "tasks - פגום 2026-10-03 09-12-40.json". נטען הגיבוי האחרון שלה מ-02/10/2026 21:05 (3 משימות). שינויים שנעשו אחרי הגיבוי הזה לא נשמרו - כדאי לבדוק את המשימות.',
        path: "C:\\Users\\User\\AppData\\Roaming\\org.tovtech.backuper",
      }
    : null;

  const snapshot = (): Snapshot => ({
    tasks,
    states,
    settings,
    autostart: true,
    drivePrompts,
    notice,
    update,
    queue: [{ taskId: "3", mode: null, trigger: "manual" }],
    current: {
      runId: "run",
      taskId: "1",
      taskName: "מסמכים ותמונות",
      mode: "incremental",
      phase: "copying",
      startedAt: iso(-2),
      sourceIndex: 2,
      sourceCount: 3,
      sourcePath: "C:\\Users\\User\\Pictures",
      filesDone: 184,
      filesTotal: 421,
      bytesDone: 940_000_000,
      bytesTotal: 2_180_000_000,
      currentFile: "C:\\Users\\User\\Pictures\\2025\\טיול צפון\\IMG_4821.jpg",
    },
  });

  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case "get_snapshot":
        return snapshot();
      case "save_tasks":
        for (const t of a.tasks as Task[]) {
          if (!t.id) t.id = String(Date.now());
          const i = tasks.findIndex((x) => x.id === t.id);
          if (i >= 0) tasks[i] = t;
          else tasks.push(t);
        }
        return a.tasks;
      case "delete_tasks":
        (a.ids as string[]).forEach((id) =>
          tasks.splice(
            tasks.findIndex((t) => t.id === id),
            1,
          ),
        );
        return null;
      case "test_sound":
        return null;
      case "save_settings":
        settings = a.settings as Settings;
        return null;
      case "get_history":
        return history;
      case "list_backups":
        return backupsOf(tasks.find((t) => t.id === a.taskId) ?? tasks[0]);
      case "folder_size":
        return String(a.path).endsWith("מלא") ? 12_345_678_901 : 48_200_000;
      case "read_log":
        return "Backuper - גיבוי אינקרמנטלי\nמקור: C:\\Users\\User\\Pictures\n\nהועתק\t1234\tC:\\Users\\User\\Pictures\\a.jpg\n";
      case "preview_schedule":
        return occurrences(a.schedule as Schedule, 3);
      case "plugin:dialog|open":
        // The file picker and the restore-from-folder picker get an answer (other folder pickers stay cancelled).
        if ((a.options as { title?: string } | undefined)?.title?.includes("רשימת המשימות")) return "K:\\Backuper";
        return (a.options as { filters?: unknown[] } | undefined)?.filters ? "C:\\Users\\User\\Desktop\\cobian.lst" : null;
      case "plugin:dialog|save":
        return `C:\\Users\\User\\Desktop\\${(a.options as { defaultPath: string }).defaultPath}`;
      case "import_tasks":
        return String(a.path).endsWith(".lst") ? { source: "cobian", tasks: cobianImport() } : snapshotImport(tasks);
      case "export_tasks":
        return a.ids ? (a.ids as string[]).length : tasks.length;
      case "dismiss_notice":
        notice = null;
        return null;
      case "list_task_snapshots":
        return [5, 60 * 26, 60 * 24 * 6].map((minutesAgo, i) => ({
          path: `${a.dir ?? "C:\\AppData"}\\task-list-backups\\tasks ${i}.json`,
          savedAt: iso(-minutesAgo),
          taskCount: tasks.length - i,
        }));
      case "answer_drive_prompts":
        drivePrompts = drivePrompts.filter((id) => !(a.ids as string[]).includes(id));
        return null;
      case "check_updates":
        update.checkedAt = new Date().toISOString();
        return update.version !== null;
      case "install_update":
        update.installWaiting = true;
        return null;
      case "run_tasks":
        return (a.ids as string[]).length;
      case "plugin:app|version":
        return "0.2.0 (dev)";
      case "plugin:event|listen":
        return 1;
      default:
        return null;
    }
  });
}

function cobianImport(): ImportedTask[] {
  const t = (id: string, name: string, extra: Partial<Task>): Task => ({ ...newTask(), id, name, ...extra });
  return [
    {
      task: t("c1", "Docs to K Daily א-ב", {
        sources: [src("C:\\Users\\User\\Documents", "Documents"), src("D:\\מסמכים", "מסמכים")],
        destination: "K:\\cobian\\Docs",
        schedule: { kind: "daily", time: "23:35" },
        fullSchedule: { kind: "weekly", days: [0], time: "23:35" },
        copyEmptyDirs: true,
      }),
      warnings: [],
      error: null,
      exists: false,
      unchanged: false,
    },
    {
      task: t("c2", "LR catalogs to K daily full", {
        sources: [src("C:\\LR catalogs", "LR catalogs")],
        destination: "K:\\Cobian LR Bkp",
        mode: "full",
        keepCount: 5,
        schedule: { kind: "daily", time: "00:10" },
        filters: [{ kind: "include", value: "*.lrcat" }],
      }),
      warnings: [],
      error: null,
      exists: false,
      unchanged: false,
    },
    {
      task: t("c3", "Pic folders to K bi-weekly ב-ו", {
        enabled: false,
        sources: [src("D:\\Pictures\\Old Pictures", "Old Pictures"), src("D:\\Pictures\\Scans and more", "Scans and more")],
        destination: "K:\\cobian\\Pic Folders",
        schedule: { kind: "weekly", days: [1, 5], time: "00:21" },
        fullSchedule: { kind: "weekly", days: [5], time: "00:21" },
      }),
      warnings: [],
      error: null,
      exists: true,
      unchanged: false,
    },
    {
      task: t("c4", "Photos to E", {
        sources: [src("D:\\Photos", "תמונות")],
        destination: "E:\\Backups",
        schedule: { kind: "daily", time: "01:00" },
      }),
      warnings: [],
      error: 'למשימות "תמונות משפחה" ו-"Photos to E" יש תיקיית מקור עם אותו שם תיקיית גיבוי (תמונות) באותו יעד - יש לשנות אחד מהם',
      exists: false,
      unchanged: false,
    },
  ];
}


/** A snapshot: the current tasks (one edited since), plus one deleted since. */
function snapshotImport(current: Task[]): ImportPreview {
  const items: ImportedTask[] = current.map((task, i) => ({
    task: i === 0 ? { ...task, keepCount: 3 } : task,
    warnings: [],
    error: null,
    exists: true,
    unchanged: i !== 0,
  }));
  items.push({
    task: { ...newTask(), id: "deleted", name: "גיבוי מסמכים ישן", sources: [{ path: "C:\\Old", folderName: "Old" }], destination: "E:\\Backups" },
    warnings: [],
    error: null,
    exists: false,
    unchanged: false,
  });
  return { source: "backuper", tasks: items };
}
