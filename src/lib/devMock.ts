// Dev-only: lets the UI run in a plain browser (`npm run dev`) with fake data,
// for visual checks and screenshots. Never loaded inside the real Tauri app.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { RunRecord, Settings, Snapshot, SourceBackups, SourceRun, Task, TaskState } from "../types";
import { newTask } from "../types";

const iso = (minutesFromNow: number) => new Date(Date.now() + minutesFromNow * 60_000).toISOString();
const src = (path: string, folderName: string) => ({ path, folderName });

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
    },
    {
      ...newTask(),
      id: "2",
      name: "תמונות משפחה",
      sources: [src("D:\\Photos", "תמונות")],
      destination: "E:\\Backups",
      mode: "full",
      keepCount: 2,
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
      nextRun: iso(9 * 60),
      lastRunAt: iso(-15 * 60),
      lastStatus: "success",
      lastMessage: "3 תיקיות, 3 הצליחו. הועתקו 132 קבצים",
    },
    "2": {
      nextRun: iso(3 * 24 * 60),
      lastRunAt: iso(-4 * 24 * 60),
      lastStatus: "warning",
      lastMessage: "נמצאו פריטים לא תואמים",
    },
    "3": { nextRun: iso(47), lastRunAt: iso(-73), lastStatus: "failed", lastMessage: "היעד לא זמין" },
    "4": { nextRun: null, lastRunAt: null, lastStatus: null, lastMessage: null },
  };
  let settings: Settings = {
    theme: "system",
    notifySuccess: true,
    notifyFailure: true,
    schedulerPaused: false,
    closeToTray: true,
    firstRunDone: true,
    globalFilters: [{ kind: "pattern", value: "~$*" }],
  };
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
        errors: ["C:\\code\\locked.db - The process cannot access the file because it is being used by another process."],
        filesFailed: 1,
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
      status: i === 3 ? "failed" : "success",
      message: sources.length > 1 ? `${sources.length} תיקיות, ${sources.length} הצליחו. הועתקו 132 קבצים` : sources[0].message,
      filesCopied: sources.reduce((n, s) => n + s.filesCopied, 0),
      bytesCopied: sources.reduce((n, s) => n + s.bytesCopied, 0),
      filesDeleted: 0,
      filesFailed: sources.reduce((n, s) => n + s.filesFailed, 0),
      sources,
    };
  });
  const backupsOf = (t: Task): SourceBackups[] =>
    t.sources.map((s) => ({
      source: s.path,
      folderName: s.folderName,
      backups: [
        {
          name: `${s.folderName} 2026-10-03 03-00 אינקרמנטלי`,
          path: `E:\\Backups\\${s.folderName} 2026-10-03 03-00 אינקרמנטלי`,
          createdAt: iso(-1 * 24 * 60),
          kind: "incremental",
          partial: false,
        },
        {
          name: `${s.folderName} 2026-10-02 03-00 אינקרמנטלי`,
          path: `E:\\Backups\\${s.folderName} 2026-10-02 03-00 אינקרמנטלי`,
          createdAt: iso(-2 * 24 * 60),
          kind: "incremental",
          partial: false,
        },
        {
          name: `${s.folderName} 2026-10-01 03-00 מלא`,
          path: `E:\\Backups\\${s.folderName} 2026-10-01 03-00 מלא`,
          createdAt: iso(-3 * 24 * 60),
          kind: "full",
          partial: false,
        },
      ],
    }));

  const snapshot = (): Snapshot => ({
    tasks,
    states,
    settings,
    autostart: true,
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
      case "save_settings":
        settings = a.settings as Settings;
        return null;
      case "get_history":
        return history;
      case "list_backups":
        return backupsOf(tasks.find((t) => t.id === a.taskId) ?? tasks[0]);
      case "folder_size":
        return String(a.path).includes("2026-10-03") ? 0 : 12_345_678_901;
      case "read_log":
        return "Backuper - גיבוי אינקרמנטלי\nמקור: C:\\Users\\User\\Pictures\n\nהועתק\t1234\tC:\\Users\\User\\Pictures\\a.jpg\n";
      case "preview_schedule":
        return [iso(60), iso(24 * 60 + 60), iso(48 * 60 + 60)];
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
