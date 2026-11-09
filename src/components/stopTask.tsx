import { api } from "../api";
import { useFeedback } from "./feedback";

/**
 * "Stop" on a running backup: asks whether to keep what was copied so far (the next run, or the
 * resume button, continues from it) or to delete it too (nothing left to continue).
 */
export function useStopTask() {
  const { choose } = useFeedback();
  return async (taskId: string, taskName: string) => {
    const answer = await choose({
      title: `לעצור את הגיבוי "${taskName}"?`,
      message: (
        <>
          מה לעשות עם מה שהועתק עד עכשיו?
          <br />
          אם תשמרו, אפשר להמשיך מאותה נקודה. אם תמחקו, ההעתקה החלקית נמחקת והריצה הבאה מתחילה מחדש (גיבוי מלא מהיר: הגיבוי הקודם
          נשמר תמיד).
        </>
      ),
      options: [
        { value: "keep", label: "עצור ושמור" },
        { value: "delete", label: "עצור ומחק", danger: true },
      ],
    });
    if (answer) await api.cancelTask(taskId, answer === "delete");
  };
}
