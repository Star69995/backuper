// Dev-only: lets the UI run in a plain browser (`npm run dev`) with fake data,
// for visual checks and screenshots. Never loaded inside the real Tauri app.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { BackupFolder, RunRecord, Settings, Snapshot, Task, TaskState } from "../types";
import { newTask } from "../types";

const iso = (minutesFromNow: number) => new Date(Date.now() + minutesFromNow * 60_000).toISOString();

export function installDevMock() {
  const tasks: Task[] = [
    { ...newTask(), id: "1", name: "מסמכים", folderName: "מסמכים", source: "C:\\Users\\User\\Documents", destination: "E:\\Backups", schedule: { kind: "daily", time: "03:00" } },
    { ...newTask(), id: "2", name: "תמונות משפחה", folderName: "תמונות", source: "D:\\Photos", destination: "E:\\Backups", mode: "full", keepCount: 2, schedule: { kind: "weekly", days: [5], time: "22:00" } },
    { ...newTask(), id: "3", name: "פרויקטים", folderName: "Projects", source: "C:\\code", destination: "F:\\Mirror", schedule: { kind: "interval", minutes: 120 }, filters: [{ kind: "folder", value: "node_modules" }, { kind: "extension", value: "tmp, log" }] },
    { ...newTask(), id: "4", name: "הגדרות תוכנות", folderName: "AppData", source: "C:\\Users\\User\\AppData\\Roaming", destination: "E:\\Backups", enabled: false, schedule: { kind: "monthly", day: 1, time: "12:00" } },
  ];
  const states: Record<string, TaskState> = {
    "1": { nextRun: iso(9 * 60), lastRunAt: iso(-15 * 60), lastStatus: "success", lastMessage: "הועתקו 132 קבצים" },
    "2": { nextRun: iso(3 * 24 * 60), lastRunAt: iso(-4 * 24 * 60), lastStatus: "warning", lastMessage: "נמצאו פריטים לא תואמים" },
    "3": { nextRun: iso(47), lastRunAt: iso(-73), lastStatus: "failed", lastMessage: "היעד לא זמין" },
    "4": { nextRun: null, lastRunAt: null, lastStatus: null, lastMessage: null },
  };
  let settings: Settings = { theme: "system", notifySuccess: true, notifyFailure: true, schedulerPaused: false, closeToTray: true, firstRunDone: true, globalFilters: [{ kind: "pattern", value: "~$*" }] };
  const history: RunRecord[] = [1, 2, 3, 4, 5, 6].map((i) => ({
    id: `r${i}`,
    taskId: String(((i - 1) % 3) + 1),
    taskName: tasks[(i - 1) % 3].name,
    trigger: i % 2 ? "scheduled" : "manual",
    mode: i === 2 ? "full" : "incremental",
    startedAt: iso(-i * 300),
    finishedAt: iso(-i * 300 + 3),
    status: i === 3 ? "failed" : i === 2 ? "warning" : "success",
    message: i === 3 ? "היעד לא זמין (F:\\Mirror): The system cannot find the path specified." : `הועתקו ${i * 37} קבצים`,
    targetFolder: "E:\\Backups\\מסמכים 2026-09-01 03-00",
    filesCopied: i * 37,
    bytesCopied: i * 48_000_000,
    filesDeleted: 0,
    filesFailed: i === 3 ? 2 : 0,
    errors: i === 3 ? ["Copying File C:\\code\\locked.db - The process cannot access the file because it is being used by another process."] : [],
    exitCode: i === 3 ? 16 : 1,
    logFile: "x",
  }));
  const backups: BackupFolder[] = [
    { name: "מסמכים 2026-09-01 03-00", path: "E:\\Backups\\מסמכים 2026-09-01 03-00", createdAt: iso(-30 * 24 * 60), partial: false },
  ];

  const snapshot = (): Snapshot => ({
    tasks,
    states,
    settings,
    autostart: true,
    queue: [{ taskId: "3", mode: null, trigger: "manual" }],
    current: {
      runId: "run",
      taskId: "2",
      taskName: "תמונות משפחה",
      mode: "full",
      phase: "copying",
      startedAt: iso(-2),
      filesDone: 1840,
      filesTotal: 4210,
      bytesDone: 9_400_000_000,
      bytesTotal: 21_800_000_000,
      currentFile: "D:\\Photos\\2025\\טיול צפון\\IMG_4821.jpg",
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
        (a.ids as string[]).forEach((id) => tasks.splice(tasks.findIndex((t) => t.id === id), 1));
        return null;
      case "save_settings":
        settings = a.settings as Settings;
        return null;
      case "get_history":
        return history;
      case "list_backups":
        return backups;
      case "folder_size":
        return 12_345_678_901;
      case "read_log":
        return "-------------------------------------------------------------------------------\n   ROBOCOPY     ::     Robust File Copy for Windows\n-------------------------------------------------------------------------------\n\t    New File  \t\t    1234\tC:\\Users\\User\\Documents\\a.txt\n";
      case "preview_schedule":
        return [iso(60), iso(24 * 60 + 60), iso(48 * 60 + 60)];
      case "run_tasks":
        return (a.ids as string[]).length;
      case "plugin:app|version":
        return "0.1.0 (dev)";
      case "plugin:event|listen":
        return 1;
      default:
        return null;
    }
  });
}
