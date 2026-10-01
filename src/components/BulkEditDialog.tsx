import { ArrowLeft, ArrowRight, Eye } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { describeSchedule, KIND_LABEL, kindFields, type TaskKind, taskKind } from "../lib/format";
import type { FilterRule, Schedule, Task } from "../types";
import FilterRulesEditor, { describeFilter } from "./FilterRulesEditor";
import ScheduleEditor from "./ScheduleEditor";
import { PathPicker } from "./TaskEditor";
import { Button, Checkbox, cx, Modal, NumberInput, PathText, Segmented, Toggle } from "./ui";

type Key =
  | "enabled"
  | "mode"
  | "destination"
  | "schedule"
  | "catchUp"
  | "keepCount"
  | "deleteBefore"
  | "deleteEmptyIncrementals"
  | "reusePrevious"
  | "copyEmptyDirs"
  | "filters"
  | "useGlobalFilters";
type Values = Pick<Task, Key | "fullSchedule">;

/** The "mode" field covers both mode and fullSchedule (full / incremental / combined). */
const valueOf = (k: Key, t: Values) => (k === "mode" ? { mode: t.mode, fullSchedule: t.fullSchedule } : t[k]);

function showKind(v: Pick<Task, "mode" | "fullSchedule">) {
  const kind = taskKind(v);
  return kind === "combined" ? `${KIND_LABEL.combined} (מלא: ${describeSchedule(v.fullSchedule!)})` : KIND_LABEL[kind];
}

const yesNo = (v: boolean) => (v ? "כן" : "לא");

const FIELDS: { key: Key; label: string; show: (v: ReturnType<typeof valueOf>) => ReactNode }[] = [
  { key: "enabled", label: "פעילה", show: (v) => yesNo(v as boolean) },
  { key: "mode", label: "סוג גיבוי", show: (v) => showKind(v as Pick<Task, "mode" | "fullSchedule">) },
  { key: "destination", label: "תיקיית יעד", show: (v) => <PathText path={v as string} /> },
  { key: "schedule", label: "תזמון", show: (v) => describeSchedule(v as Schedule) },
  { key: "catchUp", label: "השלמת גיבוי שהוחמץ", show: (v) => yesNo(v as boolean) },
  { key: "keepCount", label: "גיבויים לשמירה", show: (v) => String(v) },
  { key: "deleteBefore", label: "מחיקת גיבויים לפני גיבוי מלא", show: (v) => yesNo(v as boolean) },
  { key: "deleteEmptyIncrementals", label: "מחיקת תיקיות אינקרמנטליות ריקות", show: (v) => yesNo(v as boolean) },
  { key: "reusePrevious", label: "גיבוי מלא מהיר", show: (v) => yesNo(v as boolean) },
  { key: "copyEmptyDirs", label: "העתקת תיקיות ריקות", show: (v) => yesNo(v as boolean) },
  {
    key: "filters",
    label: "כללי סינון",
    show: (v) => ((v as FilterRule[]).length === 0 ? "ללא" : (v as FilterRule[]).map(describeFilter).join("; ")),
  },
  { key: "useGlobalFilters", label: "החלת כללי סינון כלליים", show: (v) => yesNo(v as boolean) },
];

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export default function BulkEditDialog({
  tasks,
  onClose,
  onApply,
}: {
  tasks: Task[];
  onClose: () => void;
  onApply: (next: Task[]) => void;
}) {
  const first = tasks[0];
  const [values, setValues] = useState<Values>({
    enabled: first.enabled,
    mode: first.mode,
    fullSchedule: first.fullSchedule,
    destination: first.destination,
    schedule: first.schedule,
    catchUp: first.catchUp,
    keepCount: first.keepCount,
    deleteBefore: first.deleteBefore,
    deleteEmptyIncrementals: first.deleteEmptyIncrementals,
    reusePrevious: first.reusePrevious,
    copyEmptyDirs: first.copyEmptyDirs,
    filters: [],
    useGlobalFilters: first.useGlobalFilters,
  });
  const [filterMode, setFilterMode] = useState<"add" | "replace">("add");
  const [active, setActive] = useState<Set<Key>>(new Set());
  const [step, setStep] = useState<"edit" | "preview">("edit");

  const set = <K extends Key>(k: K, v: Values[K]) => {
    setValues((x) => ({ ...x, [k]: v }));
    setActive((a) => new Set(a).add(k));
  };
  const toggleActive = (k: Key, on: boolean) =>
    setActive((a) => {
      const n = new Set(a);
      if (on) n.add(k);
      else n.delete(k);
      return n;
    });

  const next = useMemo(
    () =>
      tasks.map((t) => {
        const n = { ...t };
        active.forEach((k) => Object.assign(n, k === "mode" ? valueOf(k, values) : { [k]: values[k] }));
        if (active.has("filters") && filterMode === "add") {
          const extra = values.filters.filter((f) => !t.filters.some((x) => same(x, f)));
          n.filters = [...t.filters, ...extra];
        }
        return n;
      }),
    [tasks, active, values, filterMode],
  );
  const activeFields = FIELDS.filter((f) => active.has(f.key));
  const changedCount = next.filter((n, i) => !same(n, tasks[i])).length;

  const control = (k: Key): ReactNode => {
    switch (k) {
      case "enabled":
      case "catchUp":
      case "reusePrevious":
      case "copyEmptyDirs":
      case "useGlobalFilters":
      case "deleteBefore":
      case "deleteEmptyIncrementals":
        return <Toggle checked={values[k]} onChange={(v) => set(k, v)} />;
      case "mode":
        return (
          <div className="flex flex-col gap-3">
            <Segmented<TaskKind>
              value={taskKind(values)}
              onChange={(kind) => {
                setValues((x) => ({ ...x, ...kindFields(kind, x.fullSchedule) }));
                setActive((a) => new Set(a).add("mode"));
              }}
              options={(["incremental", "combined", "full"] as const).map((k) => ({ value: k, label: KIND_LABEL[k] }))}
            />
            {values.fullSchedule && values.mode === "incremental" && (
              <div className="flex flex-col gap-2">
                <span className="text-[13px] font-medium">תזמון הגיבוי המלא</span>
                <ScheduleEditor
                  value={values.fullSchedule}
                  onChange={(v) => {
                    setValues((x) => ({ ...x, fullSchedule: v }));
                    setActive((a) => new Set(a).add("mode"));
                  }}
                  allowManual={false}
                />
              </div>
            )}
          </div>
        );
      case "destination":
        return (
          <PathPicker
            value={values.destination}
            onChange={(v) => set("destination", v)}
            title="תיקיית יעד"
            placeholder="D:\Backups"
          />
        );
      case "schedule":
        return <ScheduleEditor value={values.schedule} onChange={(v) => set("schedule", v)} />;
      case "keepCount":
        return <NumberInput className="w-24" min={1} max={100} value={values.keepCount} onChange={(v) => set("keepCount", v)} />;
      case "filters":
        return (
          <div className="flex flex-col gap-3">
            <Segmented
              value={filterMode}
              onChange={(m) => {
                setFilterMode(m);
                setActive((a) => new Set(a).add("filters"));
              }}
              options={[
                { value: "add", label: "הוספה לכללים הקיימים" },
                { value: "replace", label: "החלפת כל הכללים" },
              ]}
            />
            <FilterRulesEditor
              rules={values.filters}
              onChange={(v) => set("filters", v)}
              emptyText={
                filterMode === "replace" ? "ללא כללים - הכללים הקיימים יימחקו." : "הוסיפו כללים שיתווספו לכל המשימות שנבחרו."
              }
            />
          </div>
        );
    }
  };

  return (
    <Modal
      size={step === "preview" ? "xl" : "lg"}
      dismissable={active.size === 0}
      title={`עריכה מרובה - ${tasks.length} משימות`}
      subtitle={
        step === "edit"
          ? "סמנו את השדות שברצונכם לשנות. שדות שלא סומנו יישארו כפי שהם בכל משימה."
          : "בדקו את השינויים לפני ההחלה."
      }
      onClose={onClose}
      footer={
        step === "edit" ? (
          <>
            <Button onClick={onClose}>ביטול</Button>
            <Button variant="primary" icon={<Eye size={16} />} disabled={active.size === 0} onClick={() => setStep("preview")}>
              תצוגה מקדימה
            </Button>
          </>
        ) : (
          <>
            <Button icon={<ArrowRight size={16} />} onClick={() => setStep("edit")} className="me-auto">
              חזרה לעריכה
            </Button>
            <Button onClick={onClose}>ביטול</Button>
            <Button variant="primary" disabled={changedCount === 0} onClick={() => onApply(next)}>
              החל על {changedCount} משימות
            </Button>
          </>
        )
      }
    >
      {step === "edit" ? (
        <div className="flex flex-col divide-y divide-line">
          {FIELDS.map((f) => {
            const on = active.has(f.key);
            const differing = new Set(tasks.map((t) => JSON.stringify(valueOf(f.key, t)))).size > 1;
            return (
              <div key={f.key} className="flex flex-col gap-3 py-3 sm:flex-row sm:items-start">
                <label className="flex w-52 shrink-0 cursor-pointer items-start gap-2 pt-1.5">
                  <span className="pt-0.5">
                    <Checkbox checked={on} onChange={(v) => toggleActive(f.key, v)} label={`לשנות ${f.label}`} />
                  </span>
                  <span className="flex flex-col">
                    <span className={cx("text-sm font-medium", !on && "text-muted")}>{f.label}</span>
                    {differing && !on && <span className="text-xs text-muted">ערכים שונים במשימות</span>}
                  </span>
                </label>
                <div className={cx("min-w-0 flex-1 transition-opacity", !on && "opacity-45")}>{control(f.key)}</div>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="overflow-x-auto rounded-lg border border-line">
          <table className="w-full text-[13px]">
            <thead>
              <tr className="border-b border-line bg-panel2 text-xs text-muted">
                <th className="px-3 py-2 text-start font-medium">משימה</th>
                {activeFields.map((f) => (
                  <th key={f.key} className="px-3 py-2 text-start font-medium">
                    {f.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {tasks.map((t, i) => (
                <tr key={t.id} className="border-b border-line last:border-b-0">
                  <td className="px-3 py-2 font-medium">{t.name}</td>
                  {activeFields.map((f) => {
                    const before = valueOf(f.key, t);
                    const after = valueOf(f.key, next[i]);
                    return (
                      <td key={f.key} className="px-3 py-2">
                        {same(before, after) ? (
                          <span className="text-muted">{f.show(before)} (ללא שינוי)</span>
                        ) : (
                          <span className="flex flex-wrap items-center gap-1.5">
                            <span className="text-muted line-through">{f.show(before)}</span>
                            <ArrowLeft size={13} className="text-muted" />
                            <span className="font-medium text-accent">{f.show(after)}</span>
                          </span>
                        )}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Modal>
  );
}
