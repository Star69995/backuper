import { AlertTriangle, FolderOpen, RotateCw, ShieldCheck } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtBytes } from "../lib/format";
import { useDiskSpace, useFullSpace } from "../lib/useDiskSpace";
import type { DeleteProblem, Snapshot } from "../types";
import { useFeedback } from "./feedback";
import { Button, Modal, PathText, SkipIcon } from "./ui";

const EXPLAIN: Record<DeleteProblem["reason"], string> = {
  access:
    'Windows לא מרשה למחוק את התיקייה: חלק מהקבצים בה שייכים לחשבון משתמש אחר או מוגנים בהרשאות של הכונן. כדי למחוק אותה צריך הרשאת מנהל - לחצו על "מחיקה בהרשאת מנהל" ואשרו בחלון של Windows שייפתח.',
  inUse: 'קובץ בתיקייה פתוח בתוכנה אחרת. סגרו אותה (גם חלון של סייר הקבצים שמציג את התיקייה) ולחצו על "ניסיון נוסף".',
  other: 'Windows לא הצליח למחוק את התיקייה. בדקו שכונן הגיבוי מחובר ולחצו על "ניסיון נוסף".',
};

const Bytes = ({ n }: { n: number }) => <bdi dir="ltr">{fmtBytes(n)}</bdi>;

/** Each folder's size, read once per `key` (null while reading or when it can't be read). */
function useFolderSizes(paths: string[], key: string): Record<string, number | null> {
  const [sizes, setSizes] = useState<Record<string, number | null>>({});
  const list = paths.join("\n");
  useEffect(() => {
    let alive = true;
    for (const p of list.split("\n")) {
      api
        .folderSize(p)
        .then((n) => alive && setSizes((s) => ({ ...s, [p]: n })))
        .catch(() => {});
    }
    return () => {
      alive = false;
    };
  }, [list, key]);
  return sizes;
}

/** How much the task's next (or running) backup is expected to take, and what that is based on. */
function expectedSize(snap: Snapshot, taskId: string, full: number | null | undefined): { bytes: number; basis: string } | null {
  const cur = snap.current?.taskId === taskId ? snap.current : null;
  if (cur && cur.mode === "full" && full) return { bytes: full, basis: "לפי הגיבוי המלא הקודם" };
  if (cur && cur.totals !== "unknown" && cur.bytesTotal > 0) {
    return { bytes: cur.bytesTotal, basis: cur.totals === "exact" ? "לפי הסריקה של הגיבוי שרץ עכשיו" : "הערכה לפי הגיבוי הקודם" };
  }
  if (full) return { bytes: full, basis: "הגיבוי המלא הבא, לפי הגיבוי המלא הקודם" };
  const last = snap.states[taskId]?.lastBackup;
  if (last && last.bytes > 0)
    return { bytes: last.bytes, basis: last.mode === "full" ? "לפי הגיבוי המלא האחרון" : "לפי הגיבוי האינקרמנטלי האחרון" };
  return null;
}

/** Free space on the destination, what the backup needs, and what deleting the stuck folders would add. */
function SpaceSummary({
  snap,
  taskId,
  stuck,
  free,
  full,
}: {
  snap: Snapshot;
  taskId: string;
  stuck: number | null;
  free: number | null | undefined;
  full: number | null | undefined;
}) {
  const need = expectedSize(snap, taskId, full);
  const short = need && typeof free === "number" && free < need.bytes;
  return (
    <div className={`flex flex-col gap-1 rounded-md p-2 text-sm ${short ? "bg-bad/10" : "bg-hover"}`}>
      <span>
        מקום פנוי בכונן הגיבוי: {free === undefined ? "בודק..." : free === null ? "הכונן לא מחובר" : <Bytes n={free} />}
        {typeof free === "number" && stuck !== null && stuck > 0 && (
          <span className="text-muted">
            {" "}
            (אחרי המחיקה: <Bytes n={free + stuck} />)
          </span>
        )}
      </span>
      <span>
        הגיבוי צפוי לתפוס:{" "}
        {need ? (
          <>
            <Bytes n={need.bytes} /> <span className="text-muted">({need.basis})</span>
          </>
        ) : (
          <span className="text-muted">אין עדיין נתונים</span>
        )}
      </span>
      {short && (
        <span className="font-medium text-bad">
          {typeof free === "number" && stuck !== null && free + stuck >= need.bytes
            ? "אין מספיק מקום - אחרי מחיקת התיקייה יהיה מספיק."
            : "אין מספיק מקום לגיבוי, גם אחרי מחיקת התיקייה. פנו מקום או שמרו פחות גיבויים."}
        </span>
      )}
    </div>
  );
}

/** "An old backup wasn't deleted" - pops up as soon as a delete fails (retention, a stopped copy, the
 * user), since the folder keeps taking room and later backups may fill the drive. */
export default function DeleteProblemDialog({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast } = useFeedback();
  const [busy, setBusy] = useState(false);
  const problems = snap.deleteProblems;
  const paths = problems.map((p) => p.path);
  const key = problems.map((p) => p.at).join();
  const sizes = useFolderSizes(paths, key);
  const taskIds = [...new Set(problems.map((p) => p.taskId))];
  const destOf = (id: string) => snap.tasks.find((t) => t.id === id)?.destination ?? "";
  const space = useDiskSpace(taskIds.map(destOf).filter(Boolean), key);
  const full = useFullSpace(taskIds, key);
  if (problems.length === 0) return null;
  // A backup waits for the answer (its delete-before failed).
  const waiting = problems.some((p) => p.blocking);

  const needsAdmin = problems.some((p) => p.reason === "access");
  const close = () => {
    api
      .dismissDeleteProblems(paths)
      .then(refresh)
      .catch((e) => toast({ tone: "bad", title: "הפעולה נכשלה", message: errorText(e) }));
  };
  const retry = (elevated: boolean) => {
    setBusy(true);
    api
      .retryDelete(paths, elevated)
      .then((left) => {
        if (left.length === 0) toast({ tone: "ok", title: paths.length === 1 ? "הגיבוי הישן נמחק" : "הגיבויים הישנים נמחקו" });
        else
          toast({ tone: "bad", title: left.length === 1 ? "תיקייה אחת עדיין לא נמחקה" : `${left.length} תיקיות עדיין לא נמחקו` });
      })
      .catch((e) => toast({ tone: "bad", title: "המחיקה לא בוצעה", message: errorText(e) }))
      .finally(() => {
        setBusy(false);
        refresh();
      });
  };

  return (
    <Modal
      size="md"
      title={
        <span className="flex items-center gap-2">
          <AlertTriangle size={18} className="text-warn" />
          {problems.length === 1 ? "גיבוי ישן לא נמחק" : "גיבויים ישנים לא נמחקו"}
        </span>
      }
      subtitle={
        waiting
          ? "הגיבוי ממתין להחלטה שלכם לפני שהוא מתחיל להעתיק: בלי המחיקה ייתכן שלא יהיה לו מספיק מקום."
          : "כל עוד הוא לא נמחק הוא תופס מקום בכונן הגיבוי, והגיבויים הבאים עלולים להיכשל כשהכונן יתמלא."
      }
      onClose={close}
      dismissable={!busy}
      footer={
        <>
          <Button onClick={close} disabled={busy} icon={waiting ? <SkipIcon size={15} /> : undefined}>
            {waiting ? "דילוג על המחיקה והמשך הגיבוי" : "לא עכשיו"}
          </Button>
          <Button
            icon={<RotateCw size={15} />}
            onClick={() => retry(false)}
            disabled={busy}
            variant={needsAdmin ? "secondary" : "primary"}
          >
            ניסיון נוסף
          </Button>
          {needsAdmin && (
            <Button variant="primary" icon={<ShieldCheck size={15} />} onClick={() => retry(true)} disabled={busy} autoFocus>
              מחיקה בהרשאת מנהל
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {taskIds.map((id) => {
          const mine = problems.filter((p) => p.taskId === id);
          const known = mine.map((p) => sizes[p.path]);
          const stuck = known.every((n) => typeof n === "number") ? (known as number[]).reduce((a, b) => a + b, 0) : null;
          return (
            <section key={id} className="flex flex-col gap-2">
              <ul className="flex flex-col gap-2">
                {mine.map((p) => (
                  <li key={p.path} className="flex flex-col gap-1.5 rounded-lg border border-line p-3">
                    <div className="flex items-start justify-between gap-2">
                      <span className="flex min-w-0 flex-col gap-0.5">
                        <span className="text-sm font-medium">{p.taskName}</span>
                        <PathText path={p.path} className="text-xs text-muted" />
                      </span>
                      <Button size="sm" variant="ghost" icon={<FolderOpen size={14} />} onClick={() => api.openPath(p.path)}>
                        פתיחת התיקייה
                      </Button>
                    </div>
                    <p className="text-sm">
                      גודל התיקייה:{" "}
                      {typeof sizes[p.path] === "number" ? (
                        <Bytes n={sizes[p.path]!} />
                      ) : (
                        <span className="text-muted">מחשב...</span>
                      )}
                    </p>
                    <p className="text-sm">{EXPLAIN[p.reason]}</p>
                    <p className="text-xs text-muted">
                      הודעת Windows: <bdi>{p.message}</bdi>
                    </p>
                  </li>
                ))}
              </ul>
              <SpaceSummary
                snap={snap}
                taskId={id}
                stuck={stuck}
                free={(() => {
                  const d = space[destOf(id)];
                  return d === undefined ? undefined : (d?.free ?? null);
                })()}
                full={(() => {
                  const f = full[id];
                  return f === undefined ? undefined : (f?.needed ?? null);
                })()}
              />
            </section>
          );
        })}
      </div>
    </Modal>
  );
}
