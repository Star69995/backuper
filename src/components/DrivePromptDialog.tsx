import { HardDrive } from "lucide-react";
import { useState } from "react";
import { api, errorText } from "../api";
import type { Snapshot } from "../types";
import { useFeedback } from "./feedback";
import { Button, Checkbox, Modal, PathText, RunIcon } from "./ui";

/** "The drive was connected - back up now?" for tasks set to ask (Task.onDriveConnect = "ask"). */
export default function DrivePromptDialog({ snap }: { snap: Snapshot }) {
  const { toast } = useFeedback();
  // Unchecked tasks; new prompts arrive checked.
  const [skipped, setSkipped] = useState<Set<string>>(new Set());
  const [remember, setRemember] = useState(false);
  const tasks = snap.drivePrompts.map((id) => snap.tasks.find((t) => t.id === id)).filter((t) => t !== undefined);
  if (tasks.length === 0) return null;

  const ids = tasks.map((t) => t.id);
  // `keep`: the answer counts as the user's choice (the close button only closes, it never remembers).
  const answer = (run: string[], keep = remember) => {
    setSkipped(new Set());
    setRemember(false);
    api
      .answerDrivePrompts(ids, run, keep)
      .then(() => {
        if (keep)
          toast({
            tone: "ok",
            title: "הבחירה נשמרה",
            message: 'אפשר לשנות אותה בעריכת המשימה, בשדה "כשהכונן מתחבר למחשב".',
          });
      })
      .catch((e) => toast({ tone: "bad", title: "הפעולה נכשלה", message: errorText(e) }));
  };
  const toRun = ids.filter((id) => !skipped.has(id));
  const one = tasks.length === 1;

  return (
    <Modal
      size="sm"
      title={
        <span className="flex items-center gap-2">
          <HardDrive size={18} className="text-accent" />
          חובר כונן
        </span>
      }
      subtitle={tasks.length === 1 ? "להתחיל את הגיבוי עכשיו?" : "אילו גיבויים להתחיל עכשיו?"}
      onClose={() => answer([], false)}
      footer={
        <>
          <Button onClick={() => answer([])}>{remember ? "לא לגבות" : "לא עכשיו"}</Button>
          <Button
            variant="primary"
            icon={<RunIcon size={15} />}
            disabled={toRun.length === 0}
            onClick={() => answer(toRun)}
            autoFocus
          >
            {tasks.length === 1 || toRun.length === tasks.length ? "גיבוי עכשיו" : `גיבוי ${toRun.length} משימות`}
          </Button>
        </>
      }
    >
      <ul className="flex flex-col gap-2">
        {tasks.map((t) => (
          <li key={t.id}>
            <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-line p-3 hover:bg-hover">
              {tasks.length > 1 && (
                <span className="pt-0.5">
                  <Checkbox
                    label={t.name}
                    checked={!skipped.has(t.id)}
                    onChange={(v) =>
                      setSkipped((s) => {
                        const n = new Set(s);
                        if (v) n.delete(t.id);
                        else n.add(t.id);
                        return n;
                      })
                    }
                  />
                </span>
              )}
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="text-sm font-medium">{t.name}</span>
                <span className="flex min-w-0 items-center gap-1 text-xs text-muted">
                  אל <PathText path={t.destination} className="truncate" />
                </span>
              </span>
            </label>
          </li>
        ))}
      </ul>
      <label className="mt-4 flex cursor-pointer items-start gap-3">
        <span className="pt-0.5">
          <Checkbox label="לזכור את הבחירה ולא לשאול שוב" checked={remember} onChange={setRemember} />
        </span>
        <span className="flex flex-col gap-0.5">
          <span className="text-sm">לזכור את הבחירה ולא לשאול שוב</span>
          {remember && (
            <span className="text-xs text-muted">
              {one
                ? "בחיבור הבא: גיבוי אוטומטי בלי לשאול אם תבחרו בגיבוי, ובלי גיבוי אם תבחרו שלא."
                : "בחיבור הבא: המשימות המסומנות יגובו אוטומטית בלי לשאול, והאחרות לא יגובו."}
            </span>
          )}
        </span>
      </label>
    </Modal>
  );
}
