import { Square } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import { fmtBytes, fmtDuration, fmtNumber, MODE_LABEL } from "../lib/format";
import type { Job, Progress, Task } from "../types";
import { Button, PathText, ProgressBar, Spinner } from "./ui";

export default function RunningCard({ progress: p, queue, tasks }: { progress: Progress; queue: Job[]; tasks: Task[] }) {
  // Re-render every second for the elapsed timer.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, []);

  const scanning = p.phase === "scanning";
  const pct = scanning ? null : p.bytesTotal > 0 ? (p.bytesDone / p.bytesTotal) * 100 : p.filesTotal > 0 ? (p.filesDone / p.filesTotal) * 100 : null;
  const queuedNames = queue.map((j) => tasks.find((t) => t.id === j.taskId)?.name).filter(Boolean);

  return (
    <section className="rounded-xl border border-accent/30 bg-panel p-4 shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <span className="flex size-9 items-center justify-center rounded-lg bg-accent-soft text-accent">
            <Spinner />
          </span>
          <div>
            <div className="font-semibold">
              מגבה: {p.taskName} <span className="text-[13px] font-normal text-muted">({MODE_LABEL[p.mode]})</span>
            </div>
            <div className="text-[13px] text-muted">
              {scanning ? "סורק את הקבצים..." : "מעתיק קבצים"} - {fmtDuration(p.startedAt)}
            </div>
          </div>
        </div>
        <Button size="sm" variant="secondary" className="text-bad" icon={<Square size={14} />} onClick={() => api.cancelTask(p.taskId)}>
          עצור
        </Button>
      </div>
      <div className="mt-4 flex flex-col gap-2">
        <ProgressBar value={pct} />
        <div className="flex flex-wrap justify-between gap-x-4 gap-y-1 text-xs text-muted">
          {scanning ? (
            <span>מחשב את כמות השינויים</span>
          ) : (
            <span>
              {fmtNumber(p.filesDone)} מתוך {fmtNumber(p.filesTotal)} קבצים, <bdi dir="ltr">{fmtBytes(p.bytesDone)}</bdi> מתוך{" "}
              <bdi dir="ltr">{fmtBytes(p.bytesTotal)}</bdi>
            </span>
          )}
          {pct !== null && <span>{Math.floor(pct)}%</span>}
        </div>
        {p.currentFile && (
          <div className="truncate text-xs text-muted">
            <PathText path={p.currentFile} />
          </div>
        )}
        {queuedNames.length > 0 && <div className="text-xs text-muted">ממתינים בתור: {queuedNames.join(", ")}</div>}
      </div>
    </section>
  );
}
