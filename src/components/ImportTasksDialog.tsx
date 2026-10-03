import { AlertTriangle, ArrowLeft, CheckCircle2, Filter, Info, XCircle } from "lucide-react";
import { type ReactNode, useState } from "react";
import { api, errorText } from "../api";
import { describeRetention, describeSchedule, KIND_LABEL, taskKind } from "../lib/format";
import type { ImportPreview, Task } from "../types";
import { useFeedback } from "./feedback";
import { describeFilter } from "./FilterRulesEditor";
import { Badge, Button, Checkbox, cx, Modal, PathText, Toggle } from "./ui";

/** "file" = picked by the user (export or Cobian), "snapshot" = an automatic task list backup. */
type Origin = { kind: "file" } | { kind: "snapshot"; label: string };

const today = () => new Date().toLocaleDateString("sv-SE"); // YYYY-MM-DD

/**
 * Import (with a preview dialog) and export of the task list, shared by the tasks view and settings.
 * Render `dialog` somewhere in the component.
 */
export function useTaskListTransfer(tasks: Task[], refresh: () => void) {
  const { toast } = useFeedback();
  const [preview, setPreview] = useState<{ data: ImportPreview; origin: Origin } | null>(null);

  const open = async (path: string, origin: Origin) => {
    try {
      setPreview({ data: await api.importTasks(path), origin });
    } catch (e) {
      toast({ tone: "bad", title: "לא ניתן לקרוא את הקובץ", message: errorText(e) });
    }
  };

  const importFile = async () => {
    const path = await api.pickFile("בחירת קובץ משימות", "רשימת משימות (Backuper או Cobian)", ["json", "lst"]);
    if (path) await open(path, { kind: "file" });
  };

  /** All tasks, or only `list`. */
  const exportTasks = async (list?: Task[]) => {
    const name = list && list.length === 1 ? list[0].name : "משימות";
    const path = await api.saveFile("ייצוא משימות", `Backuper - ${name} ${today()}.json`, "רשימת משימות", ["json"]);
    if (!path) return;
    try {
      const n = await api.exportTasks(path, list ? list.map((t) => t.id) : null);
      toast({ tone: "ok", title: n === 1 ? "משימה אחת יוצאה" : `${n} משימות יוצאו`, message: <PathText path={path} /> });
    } catch (e) {
      toast({ tone: "bad", title: "הייצוא נכשל", message: errorText(e) });
    }
  };

  /** Saves imported tasks; undo deletes the new ones and restores the ones that were updated. */
  const save = async (list: Task[]) => {
    const before = tasks.filter((t) => list.some((n) => n.id === t.id));
    const added = list.filter((n) => !before.some((t) => t.id === n.id)).map((t) => t.id);
    try {
      await api.saveTasks(list);
      setPreview(null);
      refresh();
      toast({
        tone: "ok",
        title: list.length === 1 ? "משימה אחת יובאה" : `${list.length} משימות יובאו`,
        action: {
          label: "ביטול",
          onClick: () =>
            Promise.all([api.deleteTasks(added), before.length ? api.saveTasks(before) : null])
              .then(() => {
                refresh();
                toast({ tone: "info", title: "הייבוא בוטל" });
              })
              .catch((e) => toast({ tone: "bad", title: "הביטול נכשל", message: errorText(e) })),
        },
      });
    } catch (e) {
      toast({ tone: "bad", title: "הייבוא נכשל", message: errorText(e) });
    }
  };

  const dialog = preview && (
    <ImportTasksDialog preview={preview.data} origin={preview.origin} onClose={() => setPreview(null)} onImport={save} />
  );
  return { importFile, importSnapshot: (path: string, label: string) => open(path, { kind: "snapshot", label }), exportTasks, dialog };
}

function Note({ children }: { children: ReactNode }) {
  return (
    <div className="flex gap-2 rounded-lg bg-accent-soft p-3 text-[13px] text-fg">
      <Info size={16} className="mt-0.5 shrink-0 text-accent" />
      <p>{children}</p>
    </div>
  );
}

/** Preview of the tasks read from a file; the checked ones get saved. */
function ImportTasksDialog({
  preview,
  origin,
  onClose,
  onImport,
}: {
  preview: ImportPreview;
  origin: Origin;
  onClose: () => void;
  onImport: (tasks: Task[]) => void;
}) {
  const items = preview.tasks;
  const cobian = preview.source === "cobian";
  // Existing tasks start unchecked, so an import doesn't overwrite later edits by accident.
  // Identical ones would change nothing, so they can't be picked at all.
  const [checked, setChecked] = useState<Set<string>>(
    () => new Set(items.filter((i) => !i.error && !i.exists).map((i) => i.task.id)),
  );
  const [asDisabled, setAsDisabled] = useState(false);
  const importable = items.filter((i) => !i.error && !i.unchanged);
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
      title={origin.kind === "snapshot" ? "שחזור רשימת המשימות" : cobian ? "ייבוא משימות מ-Cobian" : "ייבוא משימות"}
      subtitle={
        origin.kind === "snapshot"
          ? `עותק שנשמר ${origin.label} · ${items.length} משימות`
          : `${items.length} משימות בקובץ`
      }
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
        {cobian && (
          <Note>
            הגיבויים הקיימים של Cobian לא מזוהים כאן, ולכן הריצה הראשונה של כל משימה תהיה גיבוי מלא. כדי שהמשימות לא
            ירוצו פעמיים, כדאי להשבית אותן ב-Cobian אחרי הייבוא.
          </Note>
        )}
        {origin.kind === "snapshot" && (
          <Note>
            משימות שנמחקו מאז מסומנות לשחזור. משימות ששונו מאז לא מסומנות - סמנו אותן כדי להחזיר אותן לגרסה הזו. משימות
            שנוצרו אחרי הגיבוי לא נמחקות.
          </Note>
        )}

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
            const pickable = !i.error && !i.unchanged;
            const on = pickable && checked.has(t.id);
            const kind = taskKind(t);
            return (
              <li
                key={t.id}
                onClick={() => pickable && toggle(t.id, !on)}
                className={cx(
                  "flex gap-3 rounded-lg border p-3",
                  pickable ? "cursor-default" : "opacity-75",
                  on ? "border-accent/40 bg-accent-soft/50" : "border-line",
                )}
              >
                <div className="pt-0.5">
                  {i.error ? (
                    <XCircle size={16} className="text-bad" />
                  ) : i.unchanged ? (
                    <CheckCircle2 size={16} className="text-ok" />
                  ) : (
                    <Checkbox label={`ייבוא ${t.name}`} checked={on} onChange={(v) => toggle(t.id, v)} />
                  )}
                </div>
                <div className="flex min-w-0 flex-1 flex-col gap-1.5 text-[13px]">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-sm font-medium">{t.name}</span>
                    <Badge tone="accent">{KIND_LABEL[kind]}</Badge>
                    {!t.enabled && <Badge>{cobian ? "מושבתת ב-Cobian" : "מושבתת"}</Badge>}
                    {i.unchanged ? (
                      <Badge tone="ok">זהה למשימה הקיימת</Badge>
                    ) : (
                      i.exists && <Badge tone="warn">קיימת - תוחלף בגרסה מהקובץ</Badge>
                    )}
                  </div>
                  <div className="text-muted">
                    {describeSchedule(t.schedule)}
                    {kind === "combined" && t.fullSchedule && <> · מלא: {describeSchedule(t.fullSchedule)}</>}
                    {" · "}
                    {describeRetention(t)}
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
                  {t.filters.length > 0 && (
                    <div className="flex gap-1.5 text-muted">
                      <Filter size={14} className="mt-0.5 shrink-0" />
                      <span>{t.filters.map(describeFilter).join("; ")}</span>
                    </div>
                  )}
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
