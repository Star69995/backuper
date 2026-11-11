import { Pause, Square } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useEta } from "../lib/eta";
import { fmtBytes, fmtDuration, fmtFinishAt, fmtNumber, fmtRemaining, MODE_LABEL } from "../lib/format";
import type { Job, Progress, Task } from "../types";
import { useStopTask } from "./stopTask";
import { Button, PathText, ProgressBar, SkipIcon, Spinner } from "./ui";

export default function RunningCard({ progress: p, queue, tasks }: { progress: Progress; queue: Job[]; tasks: Task[] }) {
  const stopTask = useStopTask();
  // Re-render every second for the elapsed timer.
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, []);

  const scanning = p.phase !== "copying";
  const phaseText = {
    scanning: "סורק את הקבצים...",
    deleting: "מוחק גיבויים קודמים לפני ההעתקה...",
    pruning: "מנקה מהגיבוי קבצים שכללי הסינון מדלגים עליהם...",
    copying: "מעתיק קבצים",
  }[p.phase];
  // No totals while copying = the pre-scan was skipped and nothing could estimate the work: counters only.
  // An estimate from the previous backup can fall short (the source grew): "X of Y" and a percentage would read wrong.
  const overshot = p.bytesDone > p.bytesTotal || p.filesDone > p.filesTotal;
  const unknownTotal = !scanning && ((p.filesTotal === 0 && p.bytesTotal === 0) || overshot);
  // Totals may be an estimate from the previous backup, so never show a finished bar mid-copy.
  const pct =
    unknownTotal || scanning
      ? null
      : p.bytesTotal > 0
        ? Math.min(99, (p.bytesDone / p.bytesTotal) * 100)
        : p.filesTotal > 0
          ? Math.min(99, (p.filesDone / p.filesTotal) * 100)
          : null;
  // Without exact totals (a scan still running, or an estimate from the last backup) the time is approximate.
  const approx = p.totals === "estimate";
  const etaRaw = useEta(p);
  const eta = unknownTotal ? null : etaRaw;
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
              {p.sourceCount > 1 && `תיקייה ${p.sourceIndex} מתוך ${p.sourceCount} - `}
              {phaseText} - {fmtDuration(p.startedAt)}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="secondary" icon={<Pause size={14} />} onClick={() => api.cancelTask(p.taskId)}>
            השהיה
          </Button>
          <Button
            size="sm"
            variant="secondary"
            className="text-bad"
            icon={<Square size={14} />}
            onClick={() => stopTask(p.taskId, p.taskName)}
          >
            עצירה
          </Button>
        </div>
      </div>
      <div className="mt-4 flex flex-col gap-2">
        {p.sourceCount > 1 && (
          <div className="truncate text-xs">
            <span className="text-muted">מקור: </span>
            <PathText path={p.sourcePath} />
          </div>
        )}
        <ProgressBar value={pct} />
        <div className="flex flex-wrap justify-between gap-x-4 gap-y-1 text-xs text-muted">
          {scanning ? (
            <span>
              {p.phase === "deleting"
                ? "מפנה מקום ביעד"
                : p.phase === "pruning"
                  ? "הכללים שונו מאז ההעתקה הקודמת בתיקייה"
                  : "מחשב את כמות השינויים"}
            </span>
          ) : unknownTotal ? (
            <span>
              הועתקו {fmtNumber(p.filesDone)} קבצים, <bdi dir="ltr">{fmtBytes(p.bytesDone)}</bdi>
            </span>
          ) : (
            <span>
              {fmtNumber(p.filesDone)} מתוך {fmtNumber(p.filesTotal)} קבצים, <bdi dir="ltr">{fmtBytes(p.bytesDone)}</bdi> מתוך{" "}
              <bdi dir="ltr">{fmtBytes(p.bytesTotal)}</bdi>
            </span>
          )}
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {p.totalsScan && p.totals === "unknown" && <span>סורק את המקור בצד כדי לחשב כמה נשאר</span>}
            <span>
              {eta !== null && (
                <>
                  {approx ? "נותרו בערך " : "נותרו כ-"}
                  {fmtRemaining(eta)} (<bdi dir="ltr">{fmtFinishAt(eta)}</bdi>)
                </>
              )}
              {pct !== null && (
                <>
                  {eta !== null && " - "}
                  <bdi dir="ltr">
                    {approx && "~"}
                    {Math.floor(pct)}%
                  </bdi>
                </>
              )}
            </span>
            {p.totalsScan && (
              <Button
                size="sm"
                variant="ghost"
                icon={<SkipIcon size={13} />}
                title="ההעתקה ממשיכה כרגיל, בלי אחוזים וזמן סיום משוער"
                onClick={() => api.skipTotalsScan()}
              >
                דילוג על החישוב
              </Button>
            )}
          </span>
        </div>
        {p.currentFile && (
          <div dir="ltr" className="truncate text-xs text-muted">
            <PathText path={p.currentFile} />
          </div>
        )}
        {!scanning && !p.currentFile && (
          <div className="text-xs text-muted">
            מחכה לקובץ הראשון להעתקה. אם התיקייה כבר מכילה קבצים מהעתקה קודמת, הם נבדקים קודם והבדיקה יכולה לקחת כמה דקות
          </div>
        )}
        {queuedNames.length > 0 && <div className="text-xs text-muted">ממתינים בתור: {queuedNames.join(", ")}</div>}
      </div>
    </section>
  );
}
