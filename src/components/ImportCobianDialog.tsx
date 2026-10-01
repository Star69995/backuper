import { AlertTriangle, ArrowLeft, Info, XCircle } from "lucide-react";
import { useState } from "react";
import { describeSchedule, KIND_LABEL, taskKind } from "../lib/format";
import type { ImportedTask, Task } from "../types";
import { Badge, Checkbox, Button, cx, Modal, PathText, Toggle } from "./ui";

/** Preview of the tasks read from a Cobian task list; the checked ones get saved. */
export default function ImportCobianDialog({
  items,
  onClose,
  onImport,
}: {
  items: ImportedTask[];
  onClose: () => void;
  onImport: (tasks: Task[]) => void;
}) {
  // Already imported tasks start unchecked, so a second import doesn't overwrite edits by accident.
  const [checked, setChecked] = useState<Set<string>>(
    () => new Set(items.filter((i) => !i.error && !i.exists).map((i) => i.task.id)),
  );
  const [asDisabled, setAsDisabled] = useState(false);
  const importable = items.filter((i) => !i.error);
  const chosen = importable.filter((i) => checked.has(i.task.id));
  const allChecked = importable.length > 0 && chosen.length === importable.length;

  const toggle = (id: string, on: boolean) =>
    setChecked((s) => {
      const n = new Set(s);
      if (on) n.add(id);
      else n.delete(id);
      return n;
    });

  return (
    <Modal
      title="ייבוא משימות מ-Cobian"
      subtitle={`נמצאו ${items.length} משימות בקובץ`}
      size="lg"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>ביטול</Button>
          <Button
            variant="primary"
            disabled={chosen.length === 0}
            onClick={() => onImport(chosen.map((i) => (asDisabled ? { ...i.task, enabled: false } : i.task)))}
          >
            {chosen.length === 1 ? "ייבוא משימה אחת" : `ייבוא ${chosen.length} משימות`}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="flex gap-2 rounded-lg bg-accent-soft p-3 text-[13px] text-fg">
          <Info size={16} className="mt-0.5 shrink-0 text-accent" />
          <p>
            הגיבויים הקיימים של Cobian לא מזוהים כאן, ולכן הריצה הראשונה של כל משימה תהיה גיבוי מלא. כדי שהמשימות לא
            ירוצו פעמיים, כדאי להשבית אותן ב-Cobian אחרי הייבוא.
          </p>
        </div>

        <Toggle
          checked={asDisabled}
          onChange={setAsDisabled}
          label="ייבוא כמשימות מושבתות"
          description="אפשר להפעיל כל משימה אחרי שבודקים אותה"
        />

        {importable.length > 1 && (
          <label className="flex items-center gap-2 text-sm text-muted">
            <Checkbox
              label="בחר הכול"
              checked={allChecked}
              indeterminate={chosen.length > 0 && !allChecked}
              onChange={(on) => setChecked(new Set(on ? importable.map((i) => i.task.id) : []))}
            />
            בחר הכול
          </label>
        )}

        <ul className="flex flex-col gap-2">
          {items.map((i) => {
            const t = i.task;
            const on = !i.error && checked.has(t.id);
            const kind = taskKind(t);
            return (
              <li
                key={t.id}
                onClick={() => !i.error && toggle(t.id, !on)}
                className={cx(
                  "flex gap-3 rounded-lg border p-3",
                  i.error ? "border-line opacity-80" : "cursor-default",
                  on ? "border-accent/40 bg-accent-soft/50" : "border-line",
                )}
              >
                <div className="pt-0.5">
                  {i.error ? (
                    <XCircle size={16} className="text-bad" />
                  ) : (
                    <Checkbox label={`ייבוא ${t.name}`} checked={on} onChange={(v) => toggle(t.id, v)} />
                  )}
                </div>
                <div className="flex min-w-0 flex-1 flex-col gap-1.5 text-[13px]">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-sm font-medium">{t.name}</span>
                    <Badge tone="accent">{KIND_LABEL[kind]}</Badge>
                    {!t.enabled && <Badge>מושבתת ב-Cobian</Badge>}
                    {i.exists && <Badge tone="warn">כבר יובאה - תעודכן</Badge>}
                  </div>
                  <div className="text-muted">
                    {describeSchedule(t.schedule)}
                    {kind === "combined" && t.fullSchedule && <> · מלא: {describeSchedule(t.fullSchedule)}</>}
                    {" · "}
                    שמירת {t.keepCount} {t.keepCount === 1 ? "גיבוי" : "גיבויים"}
                  </div>
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <div className="flex min-w-0 flex-col">
                      {t.sources.map((s) => (
                        <PathText key={s.path} path={s.path} className="truncate" />
                      ))}
                    </div>
                    {t.destination && (
                      <>
                        <ArrowLeft size={14} className="shrink-0 text-muted" />
                        <PathText path={t.destination} className="min-w-0 truncate" />
                      </>
                    )}
                  </div>
                  {i.warnings.map((w) => (
                    <div key={w} className="flex gap-1.5 text-warn">
                      <AlertTriangle size={14} className="mt-0.5 shrink-0" />
                      <span>{w}</span>
                    </div>
                  ))}
                  {i.error && (
                    <div className="flex gap-1.5 text-bad">
                      <XCircle size={14} className="mt-0.5 shrink-0" />
                      <span>לא ניתן לייבא: {i.error}</span>
                    </div>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      </div>
    </Modal>
  );
}
