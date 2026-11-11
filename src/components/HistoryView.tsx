import { ArrowUp, Check, CircleAlert, Copy, EyeOff, FileText, Folder, FolderOpen, History, Info, RotateCcw, Search, Trash2 } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { api, errorText } from "../api";
import { fmtBytes, fmtDateTime, fmtDuration, fmtNumber, fmtSmart, MODE_LABEL, STATUS_LABEL, TRIGGER_LABEL } from "../lib/format";
import { cacheFolderIn } from "../lib/cacheFolders";
import { type ErrorKind, groupFileErrors } from "../lib/fileErrors";
import type { FileError, FilterRule, RunRecord, RunStatus, Snapshot, SourceRun, Task } from "../types";
import { useFeedback } from "./feedback";
import { describeFilter } from "./FilterRulesEditor";
import { useResumeTask } from "./resumeTask";
import { statusBadge } from "./TasksView";
import { AutoGrowInput, Badge, Button, cx, EmptyState, IconButton, Modal, PathText, RunIcon, Select, Spinner, TextInput } from "./ui";

export default function HistoryView({ snap }: { snap: Snapshot }) {
  const { toast, confirm } = useFeedback();
  const [history, setHistory] = useState<RunRecord[] | null>(null);
  const [taskFilter, setTaskFilter] = useState("");
  const [statusFilter, setStatusFilter] = useState<RunStatus | "">("");
  const [details, setDetails] = useState<RunRecord | null>(null);

  // The snapshot changes whenever a run finishes, so refetch then.
  useEffect(() => {
    api.history().then(setHistory);
  }, [snap]);

  const taskNames = useMemo(() => {
    const m = new Map<string, string>();
    history?.forEach((r) => m.set(r.taskId, r.taskName));
    snap.tasks.forEach((t) => m.set(t.id, t.name));
    return [...m.entries()];
  }, [history, snap.tasks]);

  const rows = (history ?? []).filter(
    (r) => (!taskFilter || r.taskId === taskFilter) && (!statusFilter || r.status === statusFilter),
  );

  const clear = async () => {
    if (
      !(await confirm({
        title: "לנקות את יומן הריצות?",
        message: "כל רשומות הריצות והיומנים המפורטים יימחקו. הגיבויים עצמם לא יושפעו.",
        confirmLabel: "ניקוי",
        danger: true,
      }))
    )
      return;
    await api.clearHistory();
    setHistory([]);
    toast({ tone: "ok", title: "היומן נוקה" });
  };

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">יומן ריצות</h1>
          <p className="text-[13px] text-muted">כל ריצות הגיבוי, כולל תוצאות ושגיאות</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Select
            className="w-44"
            value={taskFilter}
            onChange={(e) => setTaskFilter(e.target.value)}
            aria-label="סינון לפי משימה"
          >
            <option value="">כל המשימות</option>
            {taskNames.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </Select>
          <Select
            className="w-40"
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value as RunStatus | "")}
            aria-label="סינון לפי תוצאה"
          >
            <option value="">כל התוצאות</option>
            {(Object.keys(STATUS_LABEL) as RunStatus[]).map((s) => (
              <option key={s} value={s}>
                {STATUS_LABEL[s]}
              </option>
            ))}
          </Select>
          {!!history?.length && (
            <Button icon={<Trash2 size={15} />} onClick={clear}>
              ניקוי
            </Button>
          )}
        </div>
      </header>

      <section className="overflow-hidden rounded-xl border border-line bg-panel">
        {history === null ? (
          <div className="flex justify-center py-12 text-muted">
            <Spinner />
          </div>
        ) : rows.length === 0 ? (
          <EmptyState icon={<History size={26} />} title={history.length ? "אין ריצות שתואמות לסינון" : "עדיין אין ריצות"}>
            {!history.length && "כאן יופיעו התוצאות של כל גיבוי שרץ."}
          </EmptyState>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[720px] text-[13px]">
              <thead>
                <tr className="border-b border-line bg-panel2 text-xs text-muted">
                  <th className="px-3 py-2.5 text-start font-medium">תוצאה</th>
                  <th className="px-2 py-2.5 text-start font-medium">משימה</th>
                  <th className="px-2 py-2.5 text-start font-medium">התחלה</th>
                  <th className="px-2 py-2.5 text-start font-medium">משך</th>
                  <th className="px-2 py-2.5 text-start font-medium">הועתקו</th>
                  <th className="px-2 py-2.5 text-start font-medium">פרטים</th>
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, 500).map((r) => (
                  <tr
                    key={r.id}
                    onClick={() => setDetails(r)}
                    className="cursor-pointer border-b border-line last:border-b-0 hover:bg-hover/60"
                  >
                    <td className="px-3 py-2.5">{statusBadge(r.status)}</td>
                    <td className="px-2 py-2.5">
                      <div className="font-medium">{r.taskName}</div>
                      <div className="text-xs text-muted">
                        {MODE_LABEL[r.mode]} - {TRIGGER_LABEL[r.trigger]}
                      </div>
                    </td>
                    <td className="px-2 py-2.5 whitespace-nowrap">{fmtSmart(r.startedAt)}</td>
                    <td className="px-2 py-2.5 whitespace-nowrap">{fmtDuration(r.startedAt, r.finishedAt)}</td>
                    <td className="px-2 py-2.5 whitespace-nowrap">
                      {fmtNumber(r.filesCopied)} קבצים
                      <div className="text-xs text-muted">
                        <bdi dir="ltr">{fmtBytes(r.bytesCopied)}</bdi>
                      </div>
                    </td>
                    <td className="max-w-80 truncate px-2 py-2.5 text-muted" title={r.message}>
                      {r.message}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      {details && <RunDetails run={details} onClose={() => setDetails(null)} />}
    </div>
  );
}

/** Error kinds where skipping the path next time makes no sense (the problem is the drive, not the file). */
const NOT_SKIPPABLE = new Set(["diskFull", "disconnected"]);

const norm = (p: string) => p.trim().replace(/\\+$/, "").toLowerCase();

/** Whether the task's own rules already leave this path out (a file rule for it, or a folder rule by full path above it). */
function isSkipped(task: Task, path: string) {
  const p = norm(path);
  const parts = p.split("\\");
  return task.filters.some(
    (r) =>
      (r.kind === "file" && norm(r.value) === p) ||
      (r.kind === "folder" &&
        (r.value.includes("\\") ? p === norm(r.value) || p.startsWith(norm(r.value) + "\\") : parts.includes(norm(r.value)))),
  );
}

/** Skipping a failed path in the next runs: the task, whether a path is already skipped, and the action. */
interface Skipper {
  isSkipped: (path: string) => boolean;
  /** `all` = every failed path of that source, to show how many errors each level would cover. */
  skip: (path: string, source: string, all: string[]) => void;
}

/** One kind of copy failure: what happened, why, what to do, and the affected files. */
function ErrorGroup({
  kind,
  errors,
  source,
  allPaths,
  skipper,
}: {
  kind: ErrorKind;
  errors: FileError[];
  source: string;
  allPaths: string[];
  skipper: Skipper | null;
}) {
  const canSkip = skipper !== null && !NOT_SKIPPABLE.has(kind.id);
  const bad = kind.tone === "bad";
  return (
    <div
      className={cx(
        "flex flex-col gap-1.5 rounded-lg border p-3 text-[13px] leading-relaxed",
        bad ? "border-bad/30 bg-bad-soft/50" : "border-warn/30 bg-warn-soft/50",
      )}
    >
      <div className={cx("flex items-center gap-2 font-semibold", bad ? "text-bad" : "text-warn")}>
        {bad ? <CircleAlert size={16} className="shrink-0" /> : <Info size={16} className="shrink-0" />}
        <span>
          {kind.title} ({fmtNumber(errors.length)})
        </span>
      </div>
      <p>
        <span className="font-medium">למה זה קרה? </span>
        {kind.why}
      </p>
      <p>
        <span className="font-medium">מה לעשות? </span>
        {kind.fix}
      </p>
      <details className="group">
        <summary className="cursor-pointer text-xs text-muted select-none hover:text-fg">הצגת הקבצים</summary>
        <ul className="mt-1.5 flex max-h-48 flex-col gap-1 overflow-y-auto text-xs">
          {errors.map((e, i) => (
            <li key={i} className="min-w-0">
              <div className="flex min-w-0 items-center gap-2">
                <PathText path={e.path} className="block min-w-0 flex-1 truncate font-mono" />
                {canSkip &&
                  e.path &&
                  (skipper.isSkipped(e.path) ? (
                    <span className="shrink-0 text-muted">ידולג בגיבויים הבאים</span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => skipper.skip(e.path, source, allPaths)}
                      className="flex shrink-0 items-center gap-1 rounded-full border border-line px-2 py-0.5 hover:bg-hover"
                    >
                      <EyeOff size={12} /> לדלג בפעמים הבאות
                    </button>
                  ))}
              </div>
              {kind.id === "other" && (
                <bdi dir="ltr" className="selectable block text-left text-muted">
                  {e.message}
                </bdi>
              )}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

/**
 * Lines of a robocopy log around its ERROR lines (the retries show whether a file went through in the end) - for the
 * first file of each error code only: the per-file errors are already listed above, and every file of a code retries
 * the same way, so more examples only repeat it.
 */
function logErrorExcerpt(log: string, limit = 80): string[] {
  const lines = log.split(/\r?\n/);
  const out: string[] = [];
  const firstPath = new Map<string, string>();
  lines.forEach((l, i) => {
    if (out.length >= limit || !l.includes(" ERROR ")) return;
    const m = /ERROR (\d+) \(0x[0-9a-f]+\) (.*)$/i.exec(l);
    if (m) {
      const first = firstPath.get(m[1]);
      if (first === undefined) firstPath.set(m[1], m[2]);
      else if (first !== m[2]) return;
    }
    out.push(...lines.slice(i, i + 4).filter((x) => x.trim()));
  });
  return out.slice(0, limit);
}

/**
 * The errors of one source as plain text (grouped by kind like on screen), for pasting into a message or a note.
 * With everything needed to debug them: the file's state when it failed, how the copy was run, the task's rules and
 * the log lines around the errors.
 */
async function errorsText(run: RunRecord, src: SourceRun, index: number, task: Task | null, globalRules: FilterRule[]) {
  const lines = [
    `${run.taskName} - ${fmtDateTime(run.startedAt)}`,
    `${src.folderName}: ${src.source}`,
    `${src.message}`,
  ];
  for (const g of groupFileErrors(src.errors)) {
    lines.push("", `${g.kind.title} (${g.errors.length})`, `למה: ${g.kind.why}`, `מה לעשות: ${g.kind.fix}`);
    for (const e of g.errors) {
      const detail = [e.code !== null ? `שגיאה ${e.code}` : "", e.message].filter(Boolean).join(" - ");
      lines.push(detail ? `${e.path}\t${detail}` : e.path);
      if (e.debug) lines.push(`    debug: ${e.debug}`);
    }
  }
  if (src.filesFailed > src.errors.length) {
    lines.push("", `(מוצגים ${src.errors.length} מתוך ${src.filesFailed} - הרשימה המלאה ביומן המפורט)`);
  }

  lines.push(
    "",
    "--- debug ---",
    `run ${run.id}, source ${index + 1} of ${run.sources.length}; ${run.mode} (requested), ${src.mode ?? "?"} (done), ${run.trigger}`,
    `status ${src.status ?? "?"}, exit code ${src.exitCode ?? "-"}, copied ${src.filesCopied} (${src.bytesCopied} bytes), failed ${src.filesFailed}, deleted ${src.filesDeleted}, errors listed ${src.errors.length}`,
    `started ${run.startedAt}, finished ${run.finishedAt}`,
  );
  if (src.targetFolder) lines.push(`target folder ${src.targetFolder}`);
  lines.push(...(src.debug ?? []));
  if (task) {
    lines.push(
      `task: copy empty folders ${task.copyEmptyDirs}, global rules ${task.useGlobalFilters}, ${task.filters.length} own rules`,
      ...task.filters.map((r) => `  rule: ${describeFilter(r)}`),
      ...(task.useGlobalFilters ? globalRules.map((r) => `  global rule: ${describeFilter(r)}`) : []),
    );
  }
  if (src.logFile) {
    try {
      const excerpt = logErrorExcerpt(await api.readLog(run.id, index));
      if (excerpt.length) lines.push("", "--- log lines around errors (first file of each error code) ---", ...excerpt);
    } catch {
      lines.push("(the detailed log could not be read)");
    }
  }
  return lines.join("\r\n");
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Older WebView / no permission: the classic way.
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    if (!ok) throw new Error("copy failed");
  }
}

function CopyErrorsButton({ text }: { text: () => Promise<string> }) {
  const { toast } = useFeedback();
  const [done, setDone] = useState(false);
  useEffect(() => {
    if (!done) return;
    const t = setTimeout(() => setDone(false), 2000);
    return () => clearTimeout(t);
  }, [done]);
  return (
    <Button
      size="sm"
      icon={done ? <Check size={14} /> : <Copy size={14} />}
      onClick={() =>
        text()
          .then(copyText)
          .then(() => setDone(true))
          .catch(() => toast({ tone: "bad", title: "ההעתקה לא הצליחה" }))
      }
    >
      {done ? "הועתק" : "העתקת השגיאות"}
    </Button>
  );
}

function SourceCard({
  run,
  src,
  index,
  onShowLog,
  skipper,
  resumable,
  task,
  globalRules,
}: {
  run: RunRecord;
  src: SourceRun;
  index: number;
  onShowLog: (i: number) => void;
  skipper: Skipper | null;
  /** The run's task as it is now and the global rules, for the debug text of "copy errors". */
  task: Task | null;
  globalRules: FilterRule[];
  /** The run was paused and can be continued ("מושהה" instead of "הופסק"). */
  resumable: boolean;
}) {
  const { toast } = useFeedback();
  return (
    <section className="flex flex-col gap-2 rounded-xl border border-line p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Folder size={16} className="shrink-0 text-accent" />
        <bdi className="font-semibold">{src.folderName}</bdi>
        {src.mode && <Badge tone={src.mode === "full" ? "accent" : "neutral"}>{MODE_LABEL[src.mode]}</Badge>}
        {src.status && statusBadge(src.status, resumable)}
        <span className="min-w-0 truncate text-xs text-muted">
          <PathText path={src.source} />
        </span>
      </div>
      <p className="selectable text-[13px] leading-relaxed">{src.message}</p>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted">
        <span>הועתקו {fmtNumber(src.filesCopied)} קבצים</span>
        <bdi dir="ltr">{fmtBytes(src.bytesCopied)}</bdi>
        {src.backupBytes > 0 && (
          <span>
            גודל הגיבוי <bdi dir="ltr">{fmtBytes(src.backupBytes)}</bdi>
          </span>
        )}
        {src.freedBytes > 0 && (
          <span>
            פונו <bdi dir="ltr">{fmtBytes(src.freedBytes)}</bdi>
          </span>
        )}
        {src.filesDeleted > 0 && <span>הוסרו מהגיבוי {fmtNumber(src.filesDeleted)} (כבר לא במקור)</span>}
        {src.filesFailed > 0 && <span className="text-bad">נכשלו {fmtNumber(src.filesFailed)}</span>}
        {src.exitCode !== null && <span>קוד robocopy: {src.exitCode}</span>}
      </div>
      {src.targetFolder && (
        <div className="flex min-w-0 items-center gap-2 text-xs">
          <span className="shrink-0 text-muted">תיקיית הגיבוי:</span>
          <PathText path={src.targetFolder} className="min-w-0 truncate" />
        </div>
      )}
      {src.errors.length > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
          <span className="text-[13px] font-semibold">קבצים שלא הועתקו</span>
          <CopyErrorsButton text={() => errorsText(run, src, index, task, globalRules)} />
        </div>
      )}
      {groupFileErrors(src.errors).map((g) => (
        <ErrorGroup
          key={g.kind.id}
          kind={g.kind}
          errors={g.errors}
          source={src.source}
          allPaths={src.errors.map((e) => e.path).filter(Boolean)}
          skipper={skipper}
        />
      ))}
      {src.filesFailed > src.errors.length && src.errors.length > 0 && (
        <p className="text-xs text-muted">
          מוצגים {fmtNumber(src.errors.length)} מתוך {fmtNumber(src.filesFailed)} - הרשימה המלאה ביומן המפורט.
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        {src.targetFolder && (
          <Button
            size="sm"
            icon={<FolderOpen size={14} />}
            onClick={() => openBackupFolder(src.targetFolder!).catch(() => toast({ tone: "bad", title: "התיקייה כבר לא קיימת" }))}
          >
            פתיחת תיקיית הגיבוי
          </Button>
        )}
        {/* A failure with no file list (e.g. robocopy rejected the command): the debug text is the only detail. */}
        {src.errors.length === 0 && src.status === "failed" && (
          <CopyErrorsButton text={() => errorsText(run, src, index, task, globalRules)} />
        )}
        {src.logFile && (
          <Button size="sm" icon={<FileText size={14} />} onClick={() => onShowLog(index)}>
            יומן מפורט
          </Button>
        )}
      </div>
    </section>
  );
}

/** Details of one run (per source: result, errors, full log). Also opened from the tasks table. */
/**
 * A run recorded by an older version names its folder with hyphens in the time ("03-00"); such
 * folders get renamed to the current format ("03;00") on the task's next run, so try that name too.
 */
function openBackupFolder(path: string): Promise<unknown> {
  const modern = path.replace(
    / (\d{4}-\d\d-\d\d) (\d\d)-(\d\d)(?:-(\d\d))?( (?:מלא|אינקרמנטלי)(?:\.partial)?)?$/,
    (_m, date: string, h: string, m: string, s: string | undefined, tag: string | undefined) =>
      ` ${date} ${h};${m}${s ? `;${s}` : ""}${tag ?? " מלא"}`,
  );
  return api.openPath(path).catch((e) => (modern !== path ? api.openPath(modern) : Promise.reject(e)));
}

export function RunDetails({ run: r, onClose }: { run: RunRecord; onClose: () => void }) {
  const { toast } = useFeedback();
  // The run's task as it is now (null = deleted since), for "skip this next time".
  const [task, setTask] = useState<Task | null>(null);
  const [globalRules, setGlobalRules] = useState<FilterRule[]>([]);
  // This run is the task's last one, it was paused, and its folder is still there: it can be continued from here.
  const [resumable, setResumable] = useState(false);
  useEffect(() => {
    api.snapshot().then((s) => {
      setTask(s.tasks.find((t) => t.id === r.taskId) ?? null);
      setGlobalRules(s.settings.globalFilters);
      const st = s.states[r.taskId];
      const busy = s.current?.taskId === r.taskId || s.queue.some((j) => j.taskId === r.taskId);
      setResumable(
        r.status === "cancelled" &&
          !!st?.resumable &&
          st.lastStatus === "cancelled" &&
          !!st.lastRunAt &&
          new Date(st.lastRunAt).getTime() === new Date(r.finishedAt).getTime() &&
          !busy,
      );
    });
  }, [r.taskId, r.status, r.finishedAt]);
  const { resume, restart } = useResumeTask();

  // The path waiting for "skip this next time" (the dialog picks the file or one of the folders above it).
  const [skipAsk, setSkipAsk] = useState<{ path: string; source: string; all: string[] } | null>(null);

  const skip = async (rule: FilterRule) => {
    setSkipAsk(null);
    try {
      const now = (await api.snapshot()).tasks.find((t) => t.id === r.taskId);
      if (!now) throw new Error("המשימה נמחקה");
      const [saved] = await api.saveTasks([{ ...now, filters: [...now.filters, rule] }]);
      setTask(saved);
      toast({
        tone: "ok",
        title: rule.kind === "file" ? "הקובץ ידולג בגיבויים הבאים" : "התיקייה תדולג בגיבויים הבאים",
        action: {
          label: "ביטול",
          onClick: () =>
            api
              .saveTasks([now])
              .then(([t]) => setTask(t))
              .catch((e) => toast({ tone: "bad", title: "הביטול לא הצליח", message: errorText(e) })),
        },
      });
    } catch (e) {
      toast({ tone: "bad", title: "לא ניתן לעדכן את המשימה", message: errorText(e) });
    }
  };
  const askSkip = (path: string, source: string, all: string[]) => {
    if (canSkipPath(path, source)) setSkipAsk({ path, source, all });
  };
  const skipper: Skipper | null = task && { isSkipped: (p) => isSkipped(task, p), skip: askSkip };

  // A failed run (e.g. destination full) can be continued while it is the task's latest run and its half-done folder is
  // still there (the continue happens in that folder; after a newer run it no longer belongs to this record).
  // A paused run is covered by `resumable` above.
  const [canResume, setCanResume] = useState(false);
  useEffect(() => {
    if (r.status !== "failed") return;
    let alive = true;
    Promise.all([api.history(), api.listBackups(r.taskId)])
      .then(([history, sources]) => {
        const latest = history.find((h) => h.taskId === r.taskId);
        const partial = sources.some((s) => s.backups.some((b) => b.partial));
        if (alive) setCanResume(latest?.id === r.id && partial);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [r.id, r.taskId, r.status]);

  const resumeFailed = async () => {
    try {
      const queued = await api.resumeTask(r.taskId);
      toast({ tone: "info", title: queued ? "הגיבוי ימשיך מאותה נקודה" : "המשימה כבר רצה או ממתינה בתור" });
      if (queued) onClose();
    } catch (e) {
      toast({ tone: "bad", title: "לא ניתן להמשיך את הגיבוי", message: errorText(e) });
    }
  };
  const [log, setLog] = useState<{ index: number; text: string } | null>(null);
  const [logFilter, setLogFilter] = useState("");

  const showLog = (index: number) =>
    api
      .readLog(r.id, index)
      .then((text) => setLog({ index, text }))
      .catch((e) => toast({ tone: "bad", title: "לא ניתן לפתוח את היומן", message: errorText(e) }));

  const filtered = useMemo(() => {
    if (log === null || !logFilter.trim()) return log?.text ?? null;
    const q = logFilter.toLowerCase();
    return log.text
      .split("\n")
      .filter((l) => l.toLowerCase().includes(q))
      .join("\n");
  }, [log, logFilter]);

  const stat = (label: string, value: ReactNode) => (
    <div className="rounded-lg bg-panel2 px-3 py-2">
      <div className="text-xs text-muted">{label}</div>
      <div className="mt-0.5 font-medium">{value}</div>
    </div>
  );

  return (
    <Modal
      size={log !== null ? "xl" : "lg"}
      title={
        <span className="flex items-center gap-2">
          {r.taskName} {statusBadge(r.status, resumable)}
        </span>
      }
      subtitle={`${fmtDateTime(r.startedAt)} - ${MODE_LABEL[r.mode]}, ${TRIGGER_LABEL[r.trigger]}`}
      onClose={onClose}
      footer={
        resumable ? (
          <>
            <Button
              variant="primary"
              icon={<RunIcon size={15} />}
              onClick={() => resume(r.taskId).then((ok) => ok && onClose())}
            >
              המשך מאותה נקודה
            </Button>
            <Button
              icon={<RotateCcw size={15} />}
              onClick={() => restart(r.taskId, r.taskName, r.mode).then((ok) => ok && onClose())}
            >
              התחלה מחדש
            </Button>
            <Button variant="ghost" onClick={onClose}>
              סגירה
            </Button>
          </>
        ) : (
          <>
            {canResume && (
              <Button variant="primary" icon={<RunIcon size={15} />} onClick={resumeFailed}>
                המשך גיבוי מאותה נקודה
              </Button>
            )}
            <Button variant={canResume ? "secondary" : "primary"} onClick={onClose}>
              סגירה
            </Button>
          </>
        )
      }
    >
      <div className="flex flex-col gap-4">
        <p className="selectable text-sm leading-relaxed">{r.message}</p>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {stat("קבצים שהועתקו", fmtNumber(r.filesCopied))}
          {stat("נפח שהועתק", <bdi dir="ltr">{fmtBytes(r.bytesCopied)}</bdi>)}
          {stat("משך", fmtDuration(r.startedAt, r.finishedAt))}
          {/* Known only once a backup folder is complete: a failed or stopped run has none to measure. */}
          {stat(
            "גודל הגיבוי",
            r.backupBytes > 0 || r.status === "success" || r.status === "warning" ? (
              <bdi dir="ltr">{fmtBytes(r.backupBytes)}</bdi>
            ) : (
              "לא הושלם"
            ),
          )}
          {stat("מקום שפונה (גיבויים קודמים)", <bdi dir="ltr">{fmtBytes(r.freedBytes)}</bdi>)}
          {stat("הוסרו מהגיבוי (כבר לא במקור)", fmtNumber(r.filesDeleted))}
          {stat("קבצים שנכשלו", fmtNumber(r.filesFailed))}
          {stat("סיום", fmtDateTime(r.finishedAt))}
        </div>
        {r.sources.map((src, i) => (
          <SourceCard
            key={i}
            run={r}
            src={src}
            index={i}
            onShowLog={showLog}
            skipper={skipper}
            resumable={resumable}
            task={task}
            globalRules={globalRules}
          />
        ))}
        {log !== null && (
          <div className="flex flex-col gap-2">
            <h3 className="text-[13px] font-semibold">
              יומן מפורט: <bdi>{r.sources[log.index]?.folderName}</bdi>
            </h3>
            <div className="relative">
              <Search size={16} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-muted" />
              <TextInput
                value={logFilter}
                onChange={(e) => setLogFilter(e.target.value)}
                placeholder="סינון שורות ביומן..."
                className="ps-9"
              />
            </div>
            <pre
              dir="ltr"
              className="selectable max-h-[50vh] overflow-auto rounded-lg border border-line bg-panel2 p-3 text-left font-mono text-xs leading-relaxed"
            >
              {filtered || "(ריק)"}
            </pre>
          </div>
        )}
      </div>
      {skipAsk && (
        <SkipDialog
          taskName={r.taskName}
          {...skipAsk}
          onSkip={skip}
          onClose={() => setSkipAsk(null)}
        />
      )}
    </Modal>
  );
}

/** Why a folder can't be skipped for this source, or null when it can (under the source, not the source or a drive). */
function folderProblem(folder: string, source: string): string | null {
  const f = norm(folder);
  const src = norm(source);
  if (!f) return "יש לכתוב נתיב של תיקייה";
  if (!/^[a-z]:\\|^\\\\/i.test(folder.trim())) return "יש לכתוב נתיב מלא, למשל C:\\תיקייה";
  if (f === src) return "זו תיקיית המקור עצמה - דילוג עליה לא ישאיר מה לגבות";
  if (!f.startsWith(src + "\\")) return "התיקייה צריכה להיות בתוך תיקיית המקור של המשימה";
  return null;
}

/** The failed path without a trailing backslash, whether it is a folder, and the folder offered by default. */
function skipTarget(path: string) {
  const dir = path.endsWith("\\");
  const clean = path.replace(/\\+$/, "");
  return { dir, clean, folder: dir ? clean : clean.slice(0, clean.lastIndexOf("\\")) };
}

/** Whether there is anything to skip for this failed path: the file itself, or its folder. */
function canSkipPath(path: string, source: string) {
  const { dir, folder } = skipTarget(path);
  return !dir || folderProblem(folder, source) === null;
}

/**
 * Picks what to skip in the next runs: just the failed file, or a folder - its own by default, editable so a higher
 * folder (or any folder in the source) can be given instead.
 */
function SkipDialog({
  taskName,
  path,
  source,
  all,
  onSkip,
  onClose,
}: {
  taskName: string;
  path: string;
  source: string;
  all: string[];
  onSkip: (rule: FilterRule) => void;
  onClose: () => void;
}) {
  const { dir, clean, folder: ownFolder } = skipTarget(path);
  // A cache folder the path is in (like .dropbox.cache): skipped by name, wherever it is.
  const cache = cacheFolderIn(path);
  const [kind, setKind] = useState<"file" | "folder" | "cache">(cache ? "cache" : dir ? "folder" : "file");
  const [folder, setFolder] = useState(ownFolder);
  const value = folder.trim().replace(/\\+$/, "");
  const problem = kind === "folder" ? folderProblem(value, source) : null;
  const ruleToAdd: FilterRule = kind === "file" ? { kind: "file", value: clean } : { kind: "folder", value: kind === "cache" ? cache! : value };
  const f = norm(value);
  const covers = all.filter((a) => norm(a) === f || norm(a).startsWith(f + "\\")).length;
  const coversFailed = norm(clean) === f || norm(clean).startsWith(f + "\\");
  const up = value.slice(0, value.lastIndexOf("\\"));
  const canGoUp = value.includes("\\") && folderProblem(up, source) === null;

  const option = (k: "file" | "folder" | "cache", label: string, body: ReactNode) => {
    const on = kind === k;
    return (
      <div
        role="radio"
        aria-checked={on}
        tabIndex={0}
        onClick={() => setKind(k)}
        onKeyDown={(e) => (e.key === " " || e.key === "Enter") && e.target === e.currentTarget && setKind(k)}
        className={cx(
          "flex min-w-0 cursor-pointer items-start gap-2.5 rounded-lg border px-3 py-2",
          on ? "border-accent bg-accent-soft" : "border-line hover:bg-hover",
        )}
      >
        <span
          className={cx("mt-1 flex size-4 shrink-0 items-center justify-center rounded-full border", on ? "border-accent" : "border-line")}
        >
          {on && <span className="size-2 rounded-full bg-accent" />}
        </span>
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="font-medium">{label}</span>
          {body}
        </span>
      </div>
    );
  };

  return (
    <Modal
      size="md"
      title="לדלג על זה בגיבויים הבאים?"
      subtitle={
        <>
          במשימה "<bdi>{taskName}</bdi>"
        </>
      }
      onClose={onClose}
      footer={
        <>
          <Button
            variant="primary"
            icon={<EyeOff size={15} />}
            disabled={problem !== null}
            onClick={() => onSkip(ruleToAdd)}
          >
            {kind === "file" ? "דילוג על הקובץ" : kind === "cache" ? "דילוג על תיקיות המטמון" : "דילוג על התיקייה"}
          </Button>
          <Button variant="ghost" onClick={onClose}>
            ביטול
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-sm leading-relaxed">
        <div role="radiogroup" className="flex flex-col gap-2">
          {cache &&
            option(
              "cache",
              `כל תיקיות המטמון "${cache}", בכל מקום`,
              <span className="text-xs text-muted">
                תיקיית מטמון נבנית מחדש על ידי התוכנה שיצרה אותה, ולכן אין בה צורך בגיבוי. הכלל חל על תיקייה בשם הזה בכל עומק בעץ.
              </span>,
            )}
          {!dir && option("file", "רק הקובץ", <PathText path={clean} className="block font-mono text-xs break-all" />)}
          {option(
            "folder",
            dir ? "התיקייה, עם כל מה שבתוכה" : "תיקייה שלמה, עם כל מה שבתוכה",
            <>
              <div className="flex items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
                <AutoGrowInput
                  dir="ltr"
                  joiner=""
                  value={folder}
                  onChange={setFolder}
                  onFocus={() => setKind("folder")}
                  className="min-w-0 flex-1 text-left font-mono"
                  aria-label="נתיב התיקייה"
                />
                <IconButton
                  label="תיקייה אחת למעלה"
                  disabled={!canGoUp}
                  onClick={() => {
                    setKind("folder");
                    setFolder(up);
                  }}
                >
                  <ArrowUp size={16} />
                </IconButton>
              </div>
              {kind === "folder" &&
                (problem ? (
                  <span className="text-xs text-bad">{problem}</span>
                ) : !coversFailed ? (
                  <span className="text-xs text-warn">התיקייה הזו לא כוללת את {dir ? "התיקייה" : "הקובץ"} שנכשל, אז השגיאה עלולה לחזור</span>
                ) : (
                  covers > 1 && <span className="text-xs text-muted">כולל {fmtNumber(covers)} מהקבצים שנכשלו בריצה הזו</span>
                ))}
            </>,
          )}
        </div>
        <p className="text-muted">אפשר לבטל את זה בכל זמן בהגדרות המשימה, בחלק "סינון".</p>
      </div>
    </Modal>
  );
}
