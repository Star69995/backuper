import { AlertTriangle, FolderOpen, RotateCw, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { api, errorText } from "../api";
import type { DeleteProblem, Snapshot } from "../types";
import { useFeedback } from "./feedback";
import { Button, Modal, PathText } from "./ui";

const EXPLAIN: Record<DeleteProblem["reason"], string> = {
  access:
    "Windows לא מרשה למחוק את התיקייה: חלק מהקבצים בה שייכים לחשבון משתמש אחר או מוגנים בהרשאות של הכונן. כדי למחוק אותה צריך הרשאת מנהל - לחצו על \"מחיקה בהרשאת מנהל\" ואשרו בחלון של Windows שייפתח.",
  inUse:
    "קובץ בתיקייה פתוח בתוכנה אחרת. סגרו אותה (גם חלון של סייר הקבצים שמציג את התיקייה) ולחצו על \"ניסיון נוסף\".",
  other: "Windows לא הצליח למחוק את התיקייה. בדקו שכונן הגיבוי מחובר ולחצו על \"ניסיון נוסף\".",
};

/** "An old backup wasn't deleted" - pops up as soon as a delete fails (retention, a stopped copy, the
 * user), since the folder keeps taking room and later backups may fill the drive. */
export default function DeleteProblemDialog({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast } = useFeedback();
  const [busy, setBusy] = useState(false);
  const problems = snap.deleteProblems;
  if (problems.length === 0) return null;

  const paths = problems.map((p) => p.path);
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
        else toast({ tone: "bad", title: left.length === 1 ? "תיקייה אחת עדיין לא נמחקה" : `${left.length} תיקיות עדיין לא נמחקו` });
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
      subtitle="כל עוד הוא לא נמחק הוא תופס מקום בכונן הגיבוי, והגיבויים הבאים עלולים להיכשל כשהכונן יתמלא."
      onClose={close}
      dismissable={!busy}
      footer={
        <>
          <Button onClick={close} disabled={busy}>
            לא עכשיו
          </Button>
          <Button icon={<RotateCw size={15} />} onClick={() => retry(false)} disabled={busy} variant={needsAdmin ? "secondary" : "primary"}>
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
      <ul className="flex flex-col gap-2">
        {problems.map((p) => (
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
            <p className="text-sm">{EXPLAIN[p.reason]}</p>
            <p className="text-xs text-muted">
              הודעת Windows: <bdi>{p.message}</bdi>
            </p>
          </li>
        ))}
      </ul>
    </Modal>
  );
}
