import { CircleAlert, FileText, Folder, FolderOpen, History, Info, Search, Trash2 } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { api, errorText } from "../api";
import { fmtBytes, fmtDateTime, fmtDuration, fmtNumber, fmtSmart, MODE_LABEL, STATUS_LABEL, TRIGGER_LABEL } from "../lib/format";
import { type ErrorKind, groupFileErrors } from "../lib/fileErrors";
import type { FileError, RunRecord, RunStatus, Snapshot, SourceRun } from "../types";
import { useFeedback } from "./feedback";
import { statusBadge } from "./TasksView";
import { Badge, Button, cx, EmptyState, Modal, PathText, Select, Spinner, TextInput } from "./ui";

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
        confirmLabel: "נקה",
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

/** One kind of copy failure: what happened, why, what to do, and the affected files. */
function ErrorGroup({ kind, errors }: { kind: ErrorKind; errors: FileError[] }) {
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
        <summary className="cursor-pointer text-xs text-muted select-none hover:text-fg">הצג את הקבצים</summary>
        <ul className="mt-1.5 flex max-h-48 flex-col gap-1 overflow-y-auto text-xs">
          {errors.map((e, i) => (
            <li key={i} className="min-w-0">
              <PathText path={e.path} className="block truncate font-mono" />
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

function SourceCard({ src, index, onShowLog }: { src: SourceRun; index: number; onShowLog: (i: number) => void }) {
  const { toast } = useFeedback();
  return (
    <section className="flex flex-col gap-2 rounded-xl border border-line p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Folder size={16} className="shrink-0 text-accent" />
        <bdi className="font-semibold">{src.folderName}</bdi>
        {src.mode && <Badge tone={src.mode === "full" ? "accent" : "neutral"}>{MODE_LABEL[src.mode]}</Badge>}
        {src.status && statusBadge(src.status)}
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
        {src.filesDeleted > 0 && <span>נמחקו מהגיבוי {fmtNumber(src.filesDeleted)}</span>}
        {src.filesFailed > 0 && <span className="text-bad">נכשלו {fmtNumber(src.filesFailed)}</span>}
        {src.exitCode !== null && <span>קוד robocopy: {src.exitCode}</span>}
      </div>
      {src.targetFolder && (
        <div className="flex min-w-0 items-center gap-2 text-xs">
          <span className="shrink-0 text-muted">תיקיית הגיבוי:</span>
          <PathText path={src.targetFolder} className="min-w-0 truncate" />
        </div>
      )}
      {groupFileErrors(src.errors).map((g) => (
        <ErrorGroup key={g.kind.id} kind={g.kind} errors={g.errors} />
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
            onClick={() => api.openPath(src.targetFolder!).catch(() => toast({ tone: "bad", title: "התיקייה כבר לא קיימת" }))}
          >
            פתח את תיקיית הגיבוי
          </Button>
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
export function RunDetails({ run: r, onClose }: { run: RunRecord; onClose: () => void }) {
  const { toast } = useFeedback();
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
          {r.taskName} {statusBadge(r.status)}
        </span>
      }
      subtitle={`${fmtDateTime(r.startedAt)} - ${MODE_LABEL[r.mode]}, ${TRIGGER_LABEL[r.trigger]}`}
      onClose={onClose}
      footer={
        <Button variant="primary" onClick={onClose}>
          סגור
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <p className="selectable text-sm leading-relaxed">{r.message}</p>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
          {stat("קבצים שהועתקו", fmtNumber(r.filesCopied))}
          {stat("נפח שהועתק", <bdi dir="ltr">{fmtBytes(r.bytesCopied)}</bdi>)}
          {stat("משך", fmtDuration(r.startedAt, r.finishedAt))}
          {stat("גודל הגיבוי", <bdi dir="ltr">{fmtBytes(r.backupBytes)}</bdi>)}
          {stat("מקום שפונה (גיבויים קודמים)", <bdi dir="ltr">{fmtBytes(r.freedBytes)}</bdi>)}
          {stat("קבצים שנמחקו מהגיבוי", fmtNumber(r.filesDeleted))}
          {stat("קבצים שנכשלו", fmtNumber(r.filesFailed))}
          {stat("סיום", fmtDateTime(r.finishedAt))}
        </div>
        {r.sources.map((src, i) => (
          <SourceCard key={i} src={src} index={i} onShowLog={showLog} />
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
    </Modal>
  );
}
