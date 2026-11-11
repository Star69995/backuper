// Dev-only: lets the UI run in a plain browser (`npm run dev`) with fake data,
// for visual checks and screenshots. Never loaded inside the real Tauri app.
import { mockIPC } from "@tauri-apps/api/mocks";
import type {
  AdoptPlan,
  RestoreFolder,
  RestorePlan,
  BackupFolder,
  FilterCheck,
  FilterProbe,
  FilterRule,
  RuleResult,
  ImportedTask,
  ImportPreview,
  ImportSplit,
  Notice,
  RunRecord,
  Schedule,
  Settings,
  Snapshot,
  SourceBackups,
  SourceRun,
  Task,
  TaskState,
  UpdateStatus,
} from "../types";
import { newTask } from "../types";

const mockCopyStart = Date.now() - 60_000;

/** Rough stand-in for filtercheck.rs (the real one uses the engine's matching and reads the disk). */
function mockFilterCheck(rules: FilterRule[], global: FilterRule[], sources: string[], probe: FilterProbe): FilterCheck {
  const empty = (): RuleResult => ({ effect: "none", detail: null });
  const out: FilterCheck = {
    kept: null,
    folder: false,
    fullPath: null,
    asFolder: null,
    reason: null,
    onDisk: null,
    notes: [],
    rules: rules.map(empty),
    globalRules: global.map(empty),
  };
  let path = probe.path.trim().replace(/\//g, "\\");
  if (!path) return out;
  out.folder = path.endsWith("\\");
  path = path.replace(/\\+$/, "");
  out.fullPath = /^([a-z]:\\|\\\\)/i.test(path) ? path : sources[0] ? `${sources[0].replace(/\\+$/, "")}\\${path}` : null;
  const base = sources.find((s) => path.toLowerCase().startsWith(s.toLowerCase().replace(/\\+$/, "") + "\\"));
  const rel = base ? path.slice(base.replace(/\\+$/, "").length + 1) : path.replace(/^[a-z]:\\/i, "");
  const parts = rel.split("\\").filter(Boolean);
  const name = parts[parts.length - 1] ?? "";
  const dirs = out.folder ? parts : parts.slice(0, -1);
  const wild = (p: string) =>
    new RegExp(
      `^${p
        .replace(/[.+^${}()|[\]\\]/g, "\\$&")
        .replace(/\*/g, ".*")
        .replace(/\?/g, ".")}$`,
      "i",
    );
  const names = (v: string) =>
    v
      .split(/[,;]/)
      .map((x) => x.trim())
      .filter(Boolean);
  const pats = (v: string) =>
    names(v).map((x) => (/[*?]/.test(x) ? x : x.startsWith(".") ? `*${x}` : x.includes(".") ? x : `*.${x}`));
  const re = (v: string) => {
    try {
      return new RegExp(v, "i");
    } catch {
      return null;
    }
  };
  const one = (r: FilterRule): RuleResult => {
    const skip = (detail: string | null = null): RuleResult => ({ effect: "skips", detail });
    switch (r.kind) {
      case "folder": {
        const hit = dirs.find((d) => wild(r.value.trim()).test(d));
        return hit ? skip(hit === name && out.folder ? null : `מדלג על התיקייה ${hit} שבדרך`) : empty();
      }
      case "regex": {
        const x = re(r.value);
        if (!r.value.trim() || !x) return { effect: "invalid", detail: "הביטוי לא תקין" };
        if (r.folders) {
          const hit = dirs.find((d) => x.test(d));
          return hit ? skip(`מדלג על התיקייה ${hit} שבדרך`) : empty();
        }
        if (out.folder) return empty();
        if (r.include)
          return x.test(name) ? { effect: "keeps", detail: null } : skip("הקובץ לא תואם לכלל, וכל מה שלא תואם לא יגובה");
        return x.test(name) ? skip() : empty();
      }
      default:
        if (out.folder) return empty();
        switch (r.kind) {
          case "include":
            return pats(r.value).some((p) => wild(p).test(name))
              ? { effect: "keeps", detail: null }
              : skip("הקובץ לא תואם לכלל, וכל מה שלא תואם לא יגובה");
          case "includeName":
            return names(r.value).some((p) => wild(p).test(name))
              ? { effect: "keeps", detail: null }
              : skip("הקובץ לא תואם לכלל, וכל מה שלא תואם לא יגובה");
          case "extension":
            return pats(r.value).some((p) => wild(p).test(name)) ? skip() : empty();
          case "pattern":
            return names(r.value).some((p) => wild(p).test(name)) ? skip() : empty();
          case "file":
            return r.value.trim().toLowerCase() === path.toLowerCase() ? skip() : empty();
          case "largerThan":
            return probe.sizeMb === null
              ? { effect: "unknown", detail: "הקובץ לא נמצא בדיסק - הזינו גודל כדי לבדוק" }
              : probe.sizeMb > r.mb
                ? skip()
                : empty();
          case "olderThan":
            return probe.ageDays === null
              ? { effect: "unknown", detail: "הקובץ לא נמצא בדיסק - הזינו לפני כמה ימים שונה" }
              : probe.ageDays > r.days
                ? skip()
                : empty();
          case "hidden":
            return probe.hidden ? skip() : empty();
          case "system":
            return probe.system ? skip() : empty();
        }
    }
  };
  out.rules = rules.map(one);
  out.globalRules = global.map(one);
  const all = [...out.rules, ...out.globalRules];
  out.kept = !all.some((r) => r.effect === "skips");
  if (!out.kept) out.reason = out.folder ? "כלל סינון מדלג על התיקייה, יחד עם כל מה שבתוכה." : "כלל סינון מדלג על הקובץ.";
  if (!base && sources.length)
    out.notes.push("הנתיב לא נמצא בתוך אף אחת מתיקיות המקור של המשימה, אז הוא נבדק כאילו מגבים את הכונן כולו.");
  // Nothing is on disk here, so a name without a trailing "\" could be a folder as well.
  if (!out.folder) out.asFolder = { ...mockFilterCheck(rules, global, sources, { ...probe, path: `${probe.path.trim()}\\` }), notes: [] };
  return out;
}
const iso = (minutesFromNow: number) => new Date(Date.now() + minutesFromNow * 60_000).toISOString();
/** When task 5 was paused: its state and its last run record must agree exactly (that is how the run details know it can be continued). */
const PAUSED_AT = iso(-25);
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
      group: "מחשב הבית",
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
      group: "מחשב הבית",
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
      group: "עבודה",
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
    {
      ...newTask(),
      id: "5",
      name: "סרטים",
      sources: [src("D:\\Videos", "Videos")],
      destination: "E:\\Backups",
      mode: "full",
      schedule: { kind: "manual" },
    },
  ];
  const states: Record<string, TaskState> = {
    "1": {
      nextRun: next(tasks[0].schedule),
      nextFullRun: next(tasks[0].fullSchedule!),
      lastRunAt: last(tasks[0].schedule),
      lastStatus: "success",
      lastMessage: "3 תיקיות, 3 הצליחו. הועתקו 132 קבצים",
      lastBackup: { at: last(tasks[0].schedule)!, bytes: 9_600_000, mode: "incremental" },
    },
    "2": {
      nextRun: next(tasks[1].schedule),
      nextFullRun: null,
      lastRunAt: last(tasks[1].schedule),
      lastStatus: "warning",
      lastMessage: "נמצאו פריטים לא תואמים",
      lastBackup: { at: last(tasks[1].schedule)!, bytes: 12_345_678_901, mode: "full" },
    },
    "3": { nextRun: iso(47), nextFullRun: null, lastRunAt: iso(-73), lastStatus: "failed", lastMessage: "היעד לא זמין",
      lastBackup: { at: iso(-60 * 24 * 8), bytes: 61_000_000_000, mode: "full" } },
    "4": { nextRun: null, nextFullRun: null, lastRunAt: null, lastStatus: null, lastMessage: null },
    // Paused half way: the "paused backups" card and the row's "continue" button.
    "5": {
      nextRun: null,
      nextFullRun: null,
      lastRunAt: PAUSED_AT,
      lastStatus: "cancelled",
      lastMessage: "הגיבוי הושהה. אפשר להמשיך מאותה נקודה",
      resumable: true,
    },
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
    updateIdleMinutes: 2,
    exactProgress: false,
    backgroundTotals: true,
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
        // ?update=prompt: as after a click on the "new version" notification.
        prompt: new URLSearchParams(location.search).get("update") === "prompt",
      }
    : {
        currentVersion: "0.3.4",
        state: "upToDate",
        version: null,
        notes: null,
        downloaded: 0,
        total: null,
        checkedAt: iso(-90),
        error: null,
        installWaiting: false,
        prompt: false,
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
    targetFolder: `E:\\Backups\\${folderName} 2026-10-01 03h00 ${i === 0 ? "מלא" : "אינקרמנטלי"}`,
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
  // Task 5 was paused twice: the card adds both parts up.
  history.unshift(
    ...[-25, -60].map((at, i): RunRecord => {
      const files = i === 0 ? 312 : 540;
      const bytes = i === 0 ? 9_800_000_000 : 17_400_000_000;
      const t = tasks[4];
      return {
        id: `p${i}`,
        taskId: t.id,
        taskName: t.name,
        trigger: "manual",
        mode: "full",
        startedAt: iso(at - 30),
        finishedAt: i === 0 ? PAUSED_AT : iso(at),
        status: "cancelled",
        message: "הגיבוי הושהה. אפשר להמשיך מאותה נקודה",
        filesCopied: files,
        bytesCopied: bytes,
        backupBytes: 0,
        freedBytes: 0,
        filesDeleted: 0,
        filesFailed: 0,
        sources: [
          {
            ...sourceRun("Videos", "D:\\Videos", 0),
            status: "cancelled",
            message: "הגיבוי הושהה",
            targetFolder: "E:\\Backups\\Videos 2026-11-11 09h00 מלא.partial",
            filesCopied: files,
            bytesCopied: bytes,
            backupBytes: 0,
            freedBytes: 0,
          },
        ],
      };
    }),
  );
  // The task's last 3 runs as dated folders, oldest one full (all full for a full-only task).
  const stamp = (at: string) => {
    const d = new Date(at);
    const p = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}h${p(d.getMinutes())}`;
  };
  const backupsOf = (t: Task): SourceBackups[] =>
    t.sources.map((s) => ({
      source: s.path,
      folderName: s.folderName,
      backups: occurrences(t.schedule, 3, true)
        .map((createdAt, i): BackupFolder => {
          const kind = t.mode === "full" || i === 2 ? "full" : "incremental";
          const name = `${s.folderName} ${stamp(createdAt)} ${kind === "full" ? "מלא" : "אינקרמנטלי"}`;
          const bytes = kind === "full" ? 12_345_678_901 : 40_000_000 + i * 310_000_000;
          return { name, path: `${t.destination}\\${name}`, createdAt, kind, partial: false, bytes };
        })
        // A failed last run leaves its half-done folder behind (the run details offer to continue in it).
        .concat(
          states[t.id]?.lastStatus === "failed"
            ? [
                {
                  name: `${s.folderName} ${stamp(states[t.id].lastRunAt!)} מלא.partial`,
                  path: `${t.destination}\\${s.folderName} ${stamp(states[t.id].lastRunAt!)} מלא.partial`,
                  createdAt: states[t.id].lastRunAt!,
                  kind: "full" as const,
                  partial: true,
                  bytes: null,
                },
              ]
            : [],
        ),
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

  // Fresh copies each time, like the real backend: the UI memoizes on array identity.
  const snapshot = (): Snapshot => ({
    tasks: [...tasks],
    states: { ...states },
    settings,
    autostart: true,
    drivePrompts,
    notice,
    update,
    cobianRenamed: Object.fromEntries(Object.entries(cobianRenamed).map(([id, f]) => [id, f.length])),
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
      // About 15 MB/s since the copy began a minute ago, so the time estimate stays steady on screen.
      bytesDone: Math.min(2_179_000_000, Math.round(15_600_000 * ((Date.now() - mockCopyStart) / 1000))),
      bytesTotal: 2_180_000_000,
      totals: "exact",
      totalsScan: false,
      copyStartedAt: new Date(mockCopyStart).toISOString(),
      expectedRate: 14_000_000,
      laterBytes: 3_400_000_000,
      earlierBytes: 1_250_000_000,
      currentFile: "C:\\Users\\User\\Pictures\\2025\\טיול צפון\\IMG_4821.jpg",
    },
  });

  const setCobianDecision = (ids: string[], takeOver: boolean) =>
    tasks.forEach((t) => {
      if (!ids.includes(t.id)) return;
      t.cobianPending = false;
      t.cobianTakeover = takeOver;
    });
  // Folders a takeover renamed, per task (?cobian in the URL: the first task has some).
  const cobianRenamed: Record<string, RestoreFolder[]> = new URLSearchParams(location.search).has("cobian")
    ? { [tasks[0].id]: cobianPlan({ ...tasks[0], enabled: true }).sources.flatMap((s) => s.folders).map((f) => ({ ...f, problem: null })) }
    : {};

  const restorePlan = (id: string, naming: { date: string; kind: string }): RestorePlan => {
    const task = tasks.find((t) => t.id === id)!;
    const recorded = cobianRenamed[id] ?? [];
    // Like the real one: recorded renames go back exactly; a task that never backed up gets names rebuilt.
    const legacy = recorded.length === 0 && !states[id]?.lastBackup;
    const folders: RestoreFolder[] = recorded.length
      ? recorded.map((f) => ({ from: f.to, to: f.from, kind: f.kind, problem: f.problem }))
      : legacy
        ? cobianPlan({ ...task, enabled: true })
            .sources.flatMap((s) => s.folders)
            .map((f) => {
              const stamp = f.from.match(/(\d{4})-(\d\d-\d\d) (\d\d);(\d\d)/)!;
              const date = naming.date.startsWith("yy") ? `${stamp[1].slice(2)}-${stamp[2]}` : `${stamp[1]}-${stamp[2]}`;
              const time = naming.date.endsWith("hms") ? `${stamp[3]};${stamp[4]};00` : `${stamp[3]};${stamp[4]}`;
              const word = f.kind === "full" ? "Full" : "Incremental";
              const tag = naming.kind === "word" ? ` ${word}` : naming.kind === "paren" ? ` (${word})` : "";
              return { from: f.to, to: `${f.from.split(" ")[0]} ${date} ${time}${tag}`, kind: f.kind, problem: null };
            })
        : [];
    return { taskId: id, destination: task.destination, folders, legacy };
  };

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
      case "disk_space":
        // E: roomy, F: (task 3's mirror drive) not connected, anything else nearly full.
        return (a.paths as string[]).map((p) =>
          /^e:/i.test(p) ? { free: 412_000_000_000, total: 1_000_000_000_000 } : /^f:/i.test(p) ? null : { free: 18_500_000_000, total: 500_000_000_000 },
        );
      case "full_space":
        // The next full backup weighs ~20 GB; a task with delete-before frees 12 GB of it first.
        return (a.taskIds as string[]).map((id) => ({
          needed: 20_000_000_000,
          freed: tasks.find((t) => t.id === id)?.deleteBefore ? 12_000_000_000 : 0,
        }));
      case "folder_size":
        return String(a.path).endsWith("מלא") ? 12_345_678_901 : 48_200_000;
      case "read_log":
        return "Backuper - גיבוי אינקרמנטלי\nמקור: C:\\Users\\User\\Pictures\n\nהועתק\t1234\tC:\\Users\\User\\Pictures\\a.jpg\n";
      case "preview_schedule":
        return occurrences(a.schedule as Schedule, 3);
      case "check_filters":
        return mockFilterCheck(
          a.rules as FilterRule[],
          a.globalRules as FilterRule[],
          a.sources as string[],
          a.probe as FilterProbe,
        );
      case "check_regex": {
        // Rough stand-in for the Rust reader: real parsing and wording live in regexcheck.rs.
        const pattern = String(a.pattern).trim();
        const sample = a.sample ? String(a.sample) : null;
        if (!pattern) return { error: null, parts: [], note: null, matched: null, matchedText: null };
        let re: RegExp;
        try {
          re = new RegExp(pattern, "i");
        } catch {
          return { error: "תחביר לא תקין (במקום ה-1 בביטוי)", parts: [], note: null, matched: null, matchedText: null };
        }
        const m = sample ? re.exec(sample) : null;
        return {
          error: null,
          parts: (pattern.match(/\\.|\[[^\]]*\][*+?]?|\([^)]*\)[*+?]?|.[*+?]?/g) ?? []).map((token) => ({
            token,
            meaning:
              token === "^"
                ? "תחילת השם"
                : token === "$"
                  ? "סוף השם"
                  : token === "."
                    ? "התאמה לכל תו (אחד)"
                    : /^\\[^a-z0-9]$/i.test(token)
                      ? `התו "${token[1]}" כפי שהוא`
                      : `התאמה ל-${token}`,
          })),
          note: /^\^|\$$/.test(pattern) ? null : "בלי ^ בהתחלה ו-$ בסוף, מספיק שהביטוי יתאים לחלק כלשהו מהשם.",
          matched: sample ? !!m : null,
          matchedText: m ? m[0] : null,
        };
      }
      case "plugin:dialog|open":
        // The file picker and the restore-from-folder picker get an answer (other folder pickers stay cancelled).
        if ((a.options as { title?: string } | undefined)?.title?.includes("רשימת המשימות")) return "K:\\Backuper";
        return (a.options as { filters?: unknown[] } | undefined)?.filters ? "C:\\Users\\User\\Desktop\\cobian.lst" : null;
      case "plugin:dialog|save":
        return `C:\\Users\\User\\Desktop\\${(a.options as { defaultPath: string }).defaultPath}`;
      case "import_tasks":
        if (String(a.path).endsWith(".lst")) return { source: "cobian", tasks: cobianImport(), simple: null, replaces: [] };
        if (String(a.path).endsWith("config.json")) return simpleImport((a.split as ImportSplit | undefined) ?? "one");
        return snapshotImport(tasks);
      case "export_filters":
        return null;
      case "import_filters":
        return {
          rules: [
            { kind: "extension", value: "tmp, log, bak" },
            { kind: "folder", value: "cache" },
            { kind: "folder", value: "node_modules" },
          ],
          skipped: 1,
        };
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
      case "set_unsaved":
        return null;
      case "dismiss_update_prompt":
        update.prompt = false;
        return null;
      case "find_cobian_lists":
        return {
          dir: "C:\\Program Files\\Cobian Reflector\\Lists",
          lists: [
            {
              path: "C:\\Program Files\\Cobian Reflector\\Lists\\MainList.lst",
              name: "MainList",
              product: "Cobian Reflector",
              modified: iso(-60 * 20),
              taskCount: 4,
              error: null,
            },
            {
              path: "C:\\Program Files\\Cobian Reflector\\Lists\\Laptop.lst",
              name: "Laptop",
              product: "Cobian Reflector",
              modified: iso(-60 * 24 * 40),
              taskCount: 2,
              error: null,
            },
          ],
        };
      case "plan_cobian_takeover":
        return (a.tasks as Task[]).map((t) => cobianPlan(t));
      case "take_over_cobian":
        setCobianDecision(a.ids as string[], true);
        return null;
      case "decline_cobian_takeover":
        setCobianDecision(a.ids as string[], false);
        return null;
      case "plan_cobian_restore":
        return (a.ids as string[]).map((id) => restorePlan(id, a.naming as { date: string; kind: string }));
      case "restore_cobian_names": {
        let renamed = 0;
        for (const id of a.ids as string[]) {
          const n = restorePlan(id, a.naming as { date: string; kind: string }).folders.length;
          renamed += n;
          delete cobianRenamed[id];
          tasks.forEach((t) => t.id === id && ((t.cobianTakeover = false), n > 0 && (t.enabled = false)));
        }
        return { renamed, errors: [] };
      }
      case "run_tasks":
        // Like the real queue: a task waiting for the Cobian decision doesn't run.
        return (a.ids as string[]).filter((id) => !tasks.find((t) => t.id === id)?.cobianPending).length;
      case "resume_task":
        if (tasks.find((t) => t.id === a.id)?.cobianPending) return false;
        states[a.id as string] = { ...states[a.id as string], resumable: false };
        return true;
      case "set_full_next":
        states[a.id as string] = { ...states[a.id as string], fullNext: a.on as boolean };
        return null;
      case "dismiss_paused":
        states[a.id as string] = { ...states[a.id as string], resumable: false };
        return null;
      case "plugin:app|version":
        return "0.2.0 (dev)";
      case "plugin:event|listen":
        return 1;
      default:
        return null;
    }
  });
}

/** Cobian's dated folders in the task's destination: a full and two incrementals per source (none for disabled tasks, unless one waits for the Cobian decision). */
function cobianPlan(t: Task): AdoptPlan {
  const day = (n: number) => {
    const d = new Date(Date.now() - n * 86_400_000);
    const p = (x: number) => String(x).padStart(2, "0");
    return { cobian: `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`, iso: d.toISOString() };
  };
  return {
    taskId: t.id,
    destination: t.destination,
    sources: t.sources.map((s, i) => ({
      source: s.path,
      folderName: s.folderName,
      warnings: [],
      leftovers:
        t.mode === "incremental" && (t.enabled || t.cobianPending)
          ? { count: 4, first: day(9 + i).iso, last: day(4).iso }
          : null,
      folders:
        t.enabled || t.cobianPending
          ? [3, 2, 1].map((n, i) => ({
              from: `${s.folderName} ${day(n).cobian} 23;35;0${i}${t.mode === "full" || i === 0 ? " (Full)" : ""}`,
              to: `${s.folderName} ${day(n).cobian} 23-35 ${t.mode === "full" || i === 0 ? "מלא" : "אינקרמנטלי"}`,
              kind: t.mode === "full" || i === 0 ? ("full" as const) : ("incremental" as const),
              createdAt: day(n).iso,
            }))
          : [],
    })),
  };
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
        group: "מסמכים",
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
      error:
        'למשימות "תמונות משפחה" ו-"Photos to E" יש תיקיית מקור עם אותו שם תיקיית גיבוי (תמונות) באותו יעד - יש לשנות אחד מהם',
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
    task: {
      ...newTask(),
      id: "deleted",
      name: "גיבוי מסמכים ישן",
      sources: [{ path: "C:\\Old", folderName: "Old" }],
      destination: "E:\\Backups",
    },
    warnings: [],
    error: null,
    exists: false,
    unchanged: false,
  });
  return { source: "backuper", tasks: items, simple: null, replaces: [] };
}

/** A plain config (`mom_config.json`): two groups, four folders, one destination. */
function simpleImport(split: ImportSplit): ImportPreview {
  const groups = [
    ["C:\\Users\\User\\Documents", "C:\\Users\\User\\Desktop"],
    ["D:\\תמונות", "D:\\Music"],
  ];
  const name = (p: string) => p.replace(/^.*\\/, "");
  const parts =
    split === "one"
      ? [{ id: "simple-config", name: "גיבוי מהקובץ", paths: groups.flat() }]
      : split === "group"
        ? groups.map((g, i) => ({ id: `simple-config-src_${i}`, name: `גיבוי מהקובץ - ${g.map(name).join(", ")}`, paths: g }))
        : groups.flat().map((p) => ({ id: `simple-config-${name(p)}`, name: name(p), paths: [p] }));
  return {
    source: "simple",
    simple: { groups: groups.length, sources: groups.flat().length },
    replaces: split === "one" ? [] : [{ id: "simple-config", name: "גיבוי מהקובץ" }],
    tasks: parts.map((p, i) => ({
      task: {
        ...newTask(),
        id: p.id,
        name: p.name,
        sources: p.paths.map((path) => src(path, name(path))),
        destination: "H:\\",
        schedule: { kind: "manual" },
      },
      warnings: i === 0 ? ["הקובץ לא כולל תזמון - המשימות ייובאו כהפעלה ידנית"] : [],
      error: null,
      exists: false,
      unchanged: false,
    })),
  };
}
