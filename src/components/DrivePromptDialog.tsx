import { HardDrive, Play } from "lucide-react";
import { useState } from "react";
import { api, errorText } from "../api";
import type { Snapshot } from "../types";
import { useFeedback } from "./feedback";
import { Button, Checkbox, Modal, PathText } from "./ui";

/** "The drive was connected - back up now?" for tasks set to ask (Task.onDriveConnect = "ask"). */
export default function DrivePromptDialog({ snap }: { snap: Snapshot }) {
  const { toast } = useFeedback();
  // Unchecked tasks; new prompts arrive checked.
  const [skipped, setSkipped] = useState<Set<string>>(new Set());
  const tasks = snap.drivePrompts.map((id) => snap.tasks.find((t) => t.id === id)).filter((t) => t !== undefined);
  if (tasks.length === 0) return null;

  const ids = tasks.map((t) => t.id);
  const answer = (run: string[]) => {
    setSkipped(new Set());
    api.answerDrivePrompts(ids, run).catch((e) => toast({ tone: "bad", title: "הפעולה נכשלה", message: errorText(e) }));
  };
  const toRun = ids.filter((id) => !skipped.has(id));

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
      onClose={() => answer([])}
      footer={
        <>
          <Button onClick={() => answer([])}>לא עכשיו</Button>
          <Button variant="primary" icon={<Play size={15} />} disabled={toRun.length === 0} onClick={() => answer(toRun)} autoFocus>
            {tasks.length === 1 || toRun.length === tasks.length ? "גבה עכשיו" : `גבה ${toRun.length} משימות`}
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
    </Modal>
  );
}
