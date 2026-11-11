import { FileText, Pause, RotateCcw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import { fmtBytes, fmtNumber, fmtSmart, MODE_LABEL } from "../lib/format";
import type { BackupMode, RunRecord, Task, TaskState } from "../types";
import { Button, RunIcon } from "./ui";

/** How far a paused backup got: what its run(s) since the last finished one copied, so a backup paused twice counts both parts. */
interface Done {
  mode: BackupMode;
  files: number;
  bytes: number;
}

function progressOf(history: RunRecord[], taskId: string): Done | null {
  const runs = history.filter((r) => r.taskId === taskId);
  if (runs[0]?.status !== "cancelled") return null;
  const mode = runs[0].mode;
  let files = 0;
  let bytes = 0;
  for (const r of runs) {
    if (r.status !== "cancelled" || r.mode !== mode) break;
    files += r.filesCopied;
    bytes += r.bytesCopied;
  }
  return { mode, files, bytes };
}

/**
 * Paused backups waiting to be continued, above the task list: what was done so far, and a clear "continue"
 * next to "start over", so continuing doesn't look like a new backup.
 */
export default function PausedCard({
  tasks,
  states,
  onResume,
  onRestart,
  onDetails,
  onDismiss,
}: {
  /** Resumable tasks that are not running or queued. */
  tasks: Task[];
  states: Record<string, TaskState>;
  onResume: (id: string) => void;
  onRestart: (task: Task, mode: BackupMode | null) => void;
  onDetails: (id: string) => void;
  onDismiss: (task: Task) => void;
}) {
  const [done, setDone] = useState<Record<string, Done | null>>({});
  // Re-read when a task gets paused again (its last run changes).
  const key = tasks.map((t) => `${t.id}@${states[t.id]?.lastRunAt ?? ""}`).join("|");
  useEffect(() => {
    if (!key) return;
    let live = true;
    api
      .history()
      .then((h) => live && setDone(Object.fromEntries(tasks.map((t) => [t.id, progressOf(h, t.id)]))))
      .catch(() => {});
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  if (tasks.length === 0) return null;
  return (
    <section className="rounded-xl border border-warn/40 bg-panel shadow-sm">
      <div className="flex items-center gap-2 border-b border-line px-4 py-2.5 text-sm">
        <Pause size={15} className="text-warn" />
        <span className="font-semibold">{tasks.length === 1 ? "גיבוי מושהה" : `${tasks.length} גיבויים מושהים`}</span>
        <span className="text-xs text-muted">ההמשך מעתיק רק את מה שעוד לא הועתק</span>
      </div>
      <ul className="divide-y divide-line">
        {tasks.map((t) => {
          const d = done[t.id];
          return (
            <li key={t.id} className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
              <div className="flex min-w-0 items-center gap-3">
                <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-warn-soft text-warn">
                  <Pause size={18} />
                </span>
                <div className="min-w-0">
                  <div className="truncate font-semibold">
                    {t.name}
                    {d && <span className="text-[13px] font-normal text-muted"> ({MODE_LABEL[d.mode]})</span>}
                  </div>
                  <div className="text-[13px] text-muted">
                    הושהה {fmtSmart(states[t.id]?.lastRunAt)}
                    {d &&
                      (d.files > 0 ? (
                        <>
                          {" "}
                          - כבר הועתקו {fmtNumber(d.files)} קבצים, <bdi dir="ltr">{fmtBytes(d.bytes)}</bdi>
                        </>
                      ) : (
                        " - לפני שהועתקו קבצים"
                      ))}
                  </div>
                </div>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button size="sm" variant="primary" icon={<RunIcon size={14} />} onClick={() => onResume(t.id)}>
                  המשך מאותה נקודה
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  icon={<RotateCcw size={14} />}
                  onClick={() => onRestart(t, d?.mode ?? null)}
                >
                  התחלה מחדש
                </Button>
                <Button size="sm" variant="ghost" icon={<FileText size={14} />} onClick={() => onDetails(t.id)}>
                  פרטים
                </Button>
                <Button size="sm" variant="ghost" icon={<X size={14} />} onClick={() => onDismiss(t)}>
                  ביטול
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
