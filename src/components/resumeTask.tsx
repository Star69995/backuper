import { api } from "../api";
import type { BackupMode } from "../types";
import { useFeedback } from "./feedback";

/** Continue / start over / cancel for a paused backup (the paused card, the task row and menu, the run details). Each returns true once done. */
export function useResumeTask() {
  const { toast, confirm, choose } = useFeedback();

  /** The paused run again, carrying on in the folder it left. */
  const resume = async (taskId: string) => {
    const queued = await api.resumeTask(taskId);
    toast({ tone: "info", title: queued ? "הגיבוי ימשיך מאותה נקודה" : "המשימה כבר רצה או ממתינה בתור" });
    return queued;
  };

  /** Gives up on continuing: a new backup from the start, in a new folder. */
  const restart = async (taskId: string, taskName: string, mode: BackupMode | null) => {
    const ok = await confirm({
      title: `להתחיל את "${taskName}" מחדש?`,
      message:
        "הגיבוי יתחיל מההתחלה בתיקייה חדשה, במקום להמשיך את המושהה. הגיבוי החלקי הישן יימחק בהמשך, אחרי שגיבוי חדש יסתיים.",
      confirmLabel: "התחלה מחדש",
    });
    if (!ok) return false;
    const n = await api.runTasks([taskId], mode, true);
    toast({ tone: "info", title: n === 0 ? "המשימה כבר רצה או ממתינה בתור" : "הגיבוי נוסף לתור" });
    return n > 0;
  };

  /** Gives up on the paused backup for good: it leaves the paused list, with or without deleting its partial folder. */
  const dismiss = async (taskId: string, taskName: string) => {
    const answer = await choose({
      title: `לבטל את הגיבוי המושהה "${taskName}"?`,
      message: (
        <>
          אחרי הביטול לא יהיה אפשר להמשיך אותו מאותה נקודה, והוא ייעלם מהרשימה.
          <br />
          אם תמחקו, ההעתקה החלקית נמחקת מהיעד (הגיבויים השלמים לא נפגעים). אם תשמרו, התיקייה נשארת ביעד והגיבוי הבא ימחק אותה אחרי שיסתיים.
        </>
      ),
      options: [
        { value: "keep", label: "ביטול ושמירת התיקייה" },
        { value: "delete", label: "ביטול ומחיקה", danger: true },
      ],
    });
    if (!answer) return false;
    try {
      await api.dismissPaused(taskId, answer === "delete");
      toast({ tone: "info", title: "הגיבוי המושהה בוטל" });
      return true;
    } catch (e) {
      toast({ tone: "bad", title: String(e) });
      return false;
    }
  };

  return { resume, restart, dismiss };
}
