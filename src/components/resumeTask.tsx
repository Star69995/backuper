import { api } from "../api";
import type { BackupMode } from "../types";
import { useFeedback } from "./feedback";

/** Continue / start over for a paused backup (the paused card, the task row and menu, the run details). Both return true once queued. */
export function useResumeTask() {
  const { toast, confirm } = useFeedback();

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
      confirmLabel: "התחל מחדש",
    });
    if (!ok) return false;
    const n = await api.runTasks([taskId], mode, true);
    toast({ tone: "info", title: n === 0 ? "המשימה כבר רצה או ממתינה בתור" : "הגיבוי נוסף לתור" });
    return n > 0;
  };

  return { resume, restart };
}
