import type { BackupMode, DriveAction, KeepMode, RunStatus, Schedule, Task, TaskState, Trigger } from "../types";

const dateTimeFmt = new Intl.DateTimeFormat("he-IL", {
  day: "2-digit",
  month: "2-digit",
  year: "numeric",
  hour: "2-digit",
  minute: "2-digit",
});
const timeFmt = new Intl.DateTimeFormat("he-IL", { hour: "2-digit", minute: "2-digit" });
const rtf = new Intl.RelativeTimeFormat("he", { numeric: "auto" });

export const WEEKDAYS_SHORT = ["א׳", "ב׳", "ג׳", "ד׳", "ה׳", "ו׳", "ש׳"];
export const WEEKDAYS = ["ראשון", "שני", "שלישי", "רביעי", "חמישי", "שישי", "שבת"];

export function fmtDateTime(iso: string | null | undefined) {
  return iso ? dateTimeFmt.format(new Date(iso)) : "-";
}

/** "היום 03:00" / "מחר 03:00" / full date. */
export function fmtSmart(iso: string | null | undefined) {
  if (!iso) return "-";
  const d = new Date(iso);
  const now = new Date();
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diffDays = Math.round((day(d) - day(now)) / 86_400_000);
  if (diffDays === 0) return `היום ${timeFmt.format(d)}`;
  if (diffDays === 1) return `מחר ${timeFmt.format(d)}`;
  if (diffDays === -1) return `אתמול ${timeFmt.format(d)}`;
  return dateTimeFmt.format(d);
}

export function fmtRelative(iso: string | null | undefined) {
  if (!iso) return "";
  const sec = (new Date(iso).getTime() - Date.now()) / 1000;
  const abs = Math.abs(sec);
  if (abs < 60) return sec >= 0 ? "בעוד פחות מדקה" : "לפני פחות מדקה";
  if (abs < 3600) return rtf.format(Math.round(sec / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(sec / 3600), "hour");
  return rtf.format(Math.round(sec / 86400), "day");
}

export function fmtBytes(n: number) {
  if (!n) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

export function fmtNumber(n: number) {
  return n.toLocaleString("he-IL");
}

export function fmtDuration(fromIso: string, toIso: string | number = Date.now()) {
  const to = typeof toIso === "number" ? toIso : new Date(toIso).getTime();
  const s = Math.max(0, Math.round((to - new Date(fromIso).getTime()) / 1000));
  if (s < 60) return `${s} שנ׳`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} דק׳ ${s % 60} שנ׳`;
  return `${Math.floor(m / 60)} שע׳ ${m % 60} דק׳`;
}

export const MODE_LABEL: Record<BackupMode, string> = { full: "מלא", incremental: "אינקרמנטלי" };

/** What a task does: only full, only incremental, or combined (full on its own schedule + incrementals). */
export type TaskKind = "full" | "incremental" | "combined";

export const KIND_LABEL: Record<TaskKind, string> = { full: "מלא", incremental: "אינקרמנטלי", combined: "משולב" };

export const taskKind = (t: Pick<Task, "mode" | "fullSchedule">): TaskKind =>
  t.mode === "full" ? "full" : t.fullSchedule ? "combined" : "incremental";

export const DEFAULT_FULL_SCHEDULE: Schedule = { kind: "weekly", days: [5], time: "03:00" };

/** The task fields that encode a kind. Keeps an existing full schedule when switching back to combined. */
export function kindFields(kind: TaskKind, prevFull: Schedule | null): Pick<Task, "mode" | "fullSchedule"> {
  if (kind === "full") return { mode: "full", fullSchedule: null };
  if (kind === "incremental") return { mode: "incremental", fullSchedule: null };
  return { mode: "incremental", fullSchedule: prevFull ?? DEFAULT_FULL_SCHEDULE };
}

/** The task's next scheduled run (earliest of its schedules) and its type. A full wins a tie. */
export function nextRunOf(t: Task, st: TaskState | undefined): { at: string; mode: BackupMode } | null {
  if (!t.enabled || !st) return null;
  const main = t.schedule.kind !== "manual" && st.nextRun ? { at: st.nextRun, mode: t.mode } : null;
  const full = t.fullSchedule && st.nextFullRun ? { at: st.nextFullRun, mode: "full" as const } : null;
  if (!main || !full) return main ?? full;
  return new Date(full.at).getTime() <= new Date(main.at).getTime() ? full : main;
}

export const KEEP_MODE_LABEL: Record<KeepMode, string> = { count: "לפי מספר", days: "לפי זמן", all: "לתמיד" };

export function describeRetention(t: Pick<Task, "keepMode" | "keepCount" | "keepDays">) {
  switch (t.keepMode) {
    case "all":
      return "כל הגיבויים נשמרים";
    case "days":
      return t.keepDays === 1 ? "גיבויים מהיום האחרון" : `גיבויים מ-${t.keepDays} הימים האחרונים`;
    default:
      return t.keepCount === 1 ? "גיבוי מלא אחד" : `${t.keepCount} גיבויים מלאים`;
  }
}

export const STATUS_LABEL: Record<RunStatus, string> = {
  success: "הצליח",
  warning: "אזהרות",
  failed: "נכשל",
  cancelled: "בוטל",
};

export const TRIGGER_LABEL: Record<Trigger, string> = {
  manual: "ידני",
  scheduled: "מתוזמן",
  catchUp: "השלמה",
  driveConnected: "חיבור כונן",
};

export const DRIVE_ACTION_LABEL: Record<DriveAction, string> = {
  off: "כלום",
  run: "גיבוי אוטומטי",
  ask: "שאלה לפני גיבוי",
};

/** The drive roots ("E:") a task needs - mirrors drives::task_drives in Rust. */
export function taskDrives(t: Pick<Task, "destination" | "sources">) {
  const out: string[] = [];
  for (const p of [t.destination, ...t.sources.map((s) => s.path)]) {
    const m = /^\s*([a-z]):/i.exec(p);
    const d = m && `${m[1].toUpperCase()}:`;
    if (d && !out.includes(d)) out.push(d);
  }
  return out;
}

export function fmtInterval(minutes: number) {
  if (minutes === 1) return "כל דקה";
  if (minutes % 1440 === 0) return minutes === 1440 ? "כל יום" : `כל ${minutes / 1440} ימים`;
  if (minutes % 60 === 0) return minutes === 60 ? "כל שעה" : `כל ${minutes / 60} שעות`;
  return `כל ${minutes} דקות`;
}

export function describeSchedule(s: Schedule) {
  switch (s.kind) {
    case "manual":
      return "ידני בלבד";
    case "once":
      return `פעם אחת: ${fmtDateTime(s.at)}`;
    case "daily":
      return `כל יום ב-${s.time}`;
    case "weekly": {
      const days = [...s.days].sort().map((d) => WEEKDAYS_SHORT[d]);
      return s.days.length === 7 ? `כל יום ב-${s.time}` : `בימים ${days.join(" ")} ב-${s.time}`;
    }
    case "monthly":
      return `כל חודש ב-${s.day} לחודש, ${s.time}`;
    case "interval":
      return fmtInterval(s.minutes);
  }
}

/** Mirrors backup::sanitize_folder_name in Rust. */
export function sanitizeFolderName(name: string) {
  return name
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .trim()
    .replace(/\.+$/, "");
}

/** "C:\Users\me\Documents" -> "Documents", "D:\" -> "D". Mirrors backup::default_folder_name in Rust. */
export function defaultFolderName(path: string) {
  const last = path.trim().replace(/\\+$/, "").split("\\").pop() ?? "";
  return sanitizeFolderName(last.replace(/:$/, ""));
}

export function exampleFolderName(prefix: string, mode: BackupMode) {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  const tag = mode === "full" ? "מלא" : "אינקרמנטלי";
  return `${sanitizeFolderName(prefix) || "גיבוי"} ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}-${p(d.getMinutes())} ${tag}`;
}

export function joinPath(root: string, name: string) {
  return root ? `${root.replace(/\\+$/, "")}\\${name}` : name;
}
