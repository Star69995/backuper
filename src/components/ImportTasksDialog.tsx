import { AlertTriangle, ArrowLeft, CheckCircle2, Filter, Info, XCircle } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { describeRetention, describeSchedule, KIND_LABEL, taskKind } from "../lib/format";
import type { AdoptPlan, ImportPreview, Task } from "../types";
import { CobianListsDialog, planFolderCount, TakeoverSummary, takeoverToast } from "./CobianDialogs";
import { useFeedback } from "./feedback";
import { describeFilter } from "./FilterRulesEditor";
import { GroupInput, groupNames } from "./groups";
import { Badge, Button, Checkbox, ChoiceCards, cx, Field, Modal, PathText, Spinner, Toggle } from "./ui";

/**
 * "file" = picked by the user (export or Cobian; `listName` = the file name, offered as the group),
 * "snapshot" = an automatic task list backup.
 */
type Origin = { kind: "file"; listName: string } | { kind: "snapshot"; label: string };

const stem = (path: string) => path.replace(/^.*[\\/]/, "").replace(/\.[^.]*$/, "");

const today = () => new Date().toLocaleDateString("sv-SE"); // YYYY-MM-DD

/**
 * Import (with a preview dialog) and export of the task list, shared by the tasks view and settings.
 * Render `dialog` somewhere in the component.
 */
export function useTaskListTransfer(tasks: Task[], refresh: () => void) {
  const { toast } = useFeedback();
  const [preview, setPreview] = useState<{ data: ImportPreview; origin: Origin } | null>(null);
  const [cobianPicker, setCobianPicker] = useState(false);

  const open = async (path: string, origin: Origin) => {
    try {
      setPreview({ data: await api.importTasks(path), origin });
    } catch (e) {
      toast({ tone: "bad", title: "לא ניתן לקרוא את הקובץ", message: errorText(e) });
    }
  };

  const importFile = async () => {
    const path = await api.pickFile("בחירת קובץ משימות", "רשימת משימות (Backuper או Cobian)", ["json", "lst"]);
    if (path) await open(path, { kind: "file", listName: stem(path) });
  };

  /** Picks one of the Cobian task lists found on this computer (or any .lst, from Cobian's folder). */
  const importCobian = () => setCobianPicker(true);

  /** All tasks, or only `list` (`name` = what the file is named after). */
  const exportTasks = async (list?: Task[], name = list && list.length === 1 ? list[0].name : "משימות") => {
    const path = await api.saveFile("ייצוא משימות", `Backuper - ${name} ${today()}.json`, "רשימת משימות", ["json"]);
    if (!path) return;
    try {
      const n = await api.exportTasks(path, list ? list.map((t) => t.id) : null);
      toast({ tone: "ok", title: n === 1 ? "משימה אחת יוצאה" : `${n} משימות יוצאו`, message: <PathText path={path} /> });
    } catch (e) {
      toast({ tone: "bad", title: "הייצוא נכשל", message: errorText(e) });
    }
  };

  /**
   * Saves imported tasks, then takes over the Cobian backups of `takeOver` (ids).
   * Undo deletes the new tasks and restores the ones that were updated (renamed folders stay).
   */
  const save = async (list: Task[], takeOver: string[]) => {
    const before = tasks.filter((t) => list.some((n) => n.id === t.id));
    const added = list.filter((n) => !before.some((t) => t.id === n.id)).map((t) => t.id);
    try {
      await api.saveTasks(list);
      setPreview(null);
      refresh();
      toast({
        tone: "ok",
        title: list.length === 1 ? "משימה אחת יובאה" : `${list.length} משימות יובאו`,
        message: list.some((t) => t.cobianPending)
          ? "המשימות מושבתות. כשתפעילו משימה תישאלו אם להמשיך מהגיבויים של Cobian."
          : undefined,
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
      return;
    }
    if (takeOver.length === 0) return;
    try {
      toast(takeoverToast(await api.takeOverCobian(takeOver)));
    } catch (e) {
      toast({
        tone: "warn",
        title: "המשימות יובאו, אבל הגיבויים של Cobian לא הומרו",
        message: `${errorText(e)}. אפשר להמיר אותם אחר כך מתפריט המשימה - "המשך מגיבויי Cobian".`,
      });
    }
  };

  const dialog = (
    <>
      {cobianPicker && (
        <CobianListsDialog
          onClose={() => setCobianPicker(false)}
          onPick={(path, listName) => {
            setCobianPicker(false);
            open(path, { kind: "file", listName });
          }}
        />
      )}
      {preview && (
        <ImportTasksDialog
          preview={preview.data}
          origin={preview.origin}
          groups={groupNames(tasks)}
          onClose={() => setPreview(null)}
          onImport={save}
        />
      )}
    </>
  );
  return {
    importFile,
    importCobian,
    importSnapshot: (path: string, label: string) => open(path, { kind: "snapshot", label }),
    exportTasks,
    dialog,
  };
}

function Note({ children }: { children: ReactNode }) {
  return (
    <div className="flex gap-2 rounded-lg bg-accent-soft p-3 text-[13px] text-fg">
      <Info size={16} className="mt-0.5 shrink-0 text-accent" />
      <p>{children}</p>
    </div>
  );
}

type Step = "review" | "options";

/** Cobian's existing backups: take them over on import, don't, or ask when a task is first switched on. */
type Takeover = "now" | "never" | "later";

/** Preview of the tasks read from a file; the checked ones get saved. */
function ImportTasksDialog({
  preview,
  origin,
  groups,
  onClose,
  onImport,
}: {
  preview: ImportPreview;
  origin: Origin;
  groups: string[];
  onClose: () => void;
  onImport: (tasks: Task[], takeOver: string[]) => void;
}) {
  const items = preview.tasks;
  const cobian = preview.source === "cobian";
  // Our own files carry each task's group, and so do Cobian's (TaskGroup). The other tasks (Cobian
  // without a group, a plain config) go into one group, named after the list file (Cobian's default
  // list, "MainList", means the general list).
  const withGroup = items.filter((i) => !i.error && i.task.group).length;
  const askGroup = preview.source !== "backuper" && items.some((i) => !i.error && !i.task.group);
  const [group, setGroup] = useState(() =>
    askGroup && origin.kind === "file" && !/^mainlist$/i.test(origin.listName) ? origin.listName : "",
  );
  // Cobian's existing backups in each task's destination, renamed on import so the tasks continue from them.
  const [plans, setPlans] = useState<Record<string, AdoptPlan> | null>(null);
  const [takeOver, setTakeOver] = useState<Takeover>("now");
  useEffect(() => {
    if (!cobian) return;
    const valid = items.filter((i) => !i.error).map((i) => i.task);
    api
      .planCobianTakeover(valid)
      .then((list) => setPlans(Object.fromEntries(list.map((p) => [p.taskId, p]))))
      .catch(() => setPlans({}));
  }, [cobian, items]);
  // Existing tasks start unchecked, so an import doesn't overwrite later edits by accident.
  // Identical ones would change nothing, so they can't be picked at all.
  const [checked, setChecked] = useState<Set<string>>(
    () => new Set(items.filter((i) => !i.error && !i.exists).map((i) => i.task.id)),
  );
  // Cobian lists are imported in two steps: review (details, warnings, errors), then the options -
  // whether to continue from Cobian's backups (only when some were found) and what to switch on. Until
  // the user has stopped Cobian, the safe default is to import the tasks switched off.
  const [step, setStep] = useState<Step>("review");
  const [asDisabled, setAsDisabled] = useState(cobian);
  const importable = items.filter((i) => !i.error && !i.unchanged);
  const chosen = importable.filter((i) => checked.has(i.task.id));
  const allChecked = importable.length > 0 && chosen.length === importable.length;
  const foundTasks = chosen.filter((i) => planFolderCount(plans?.[i.task.id]) > 0);
  const steps: Step[] = cobian ? ["review", "options"] : [];
  // "Decide later" imports everything switched off and asks when a task is switched on.
  const later = cobian && foundTasks.length > 0 && takeOver === "later";
  const pos = steps.indexOf(step);
  const prev = steps[pos - 1];
  const next = steps[pos + 1];
  // Tasks that were already switched off in Cobian stay off when switching the rest on.
  const offInCobian = chosen.filter((i) => !i.task.enabled).length;

  const doImport = () =>
    onImport(
      chosen.map((i) => ({
        ...i.task,
        ...((asDisabled || later) && { enabled: false }),
        ...(later && planFolderCount(plans?.[i.task.id]) > 0 && { cobianPending: true }),
        ...(askGroup && !i.task.group && { group: group.trim() }),
      })),
      cobian && takeOver === "now" ? foundTasks.map((i) => i.task.id) : [],
    );
  const importLabel = chosen.length === 1 ? "ייבוא משימה אחת" : `ייבוא ${chosen.length} משימות`;

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
          : cobian
            ? `${items.length} משימות בקובץ · שלב ${pos + 1} מתוך ${steps.length}`
            : `${items.length} משימות בקובץ`
      }
      size="lg"
      onClose={onClose}
      footer={
        <>
          {prev ? <Button onClick={() => setStep(prev)}>חזרה</Button> : <Button onClick={onClose}>ביטול</Button>}
          <Button
            variant="primary"
            disabled={chosen.length === 0 || (cobian && !plans)}
            onClick={next ? () => setStep(next) : doImport}
          >
            {next ? "המשך" : importLabel}
          </Button>
        </>
      }
    >
      {step === "options" ? (
        <OptionsStep
          count={chosen.length}
          offInCobian={offInCobian}
          asDisabled={asDisabled}
          onAsDisabled={setAsDisabled}
          found={foundTasks.map((i) => i.task)}
          plans={plans}
          takeOver={takeOver}
          onTakeOver={setTakeOver}
        />
      ) : (
        <div className="flex flex-col gap-4">
          {origin.kind === "snapshot" && (
            <Note>
              משימות שנמחקו מאז מסומנות לשחזור. משימות ששונו מאז לא מסומנות - סמנו אותן כדי להחזיר אותן לגרסה הזו. משימות שנוצרו
              אחרי הגיבוי לא נמחקות.
            </Note>
          )}

          {cobian && !plans && (
            <div className="flex items-center gap-2 text-[13px] text-muted">
              <Spinner className="size-4" /> מחפש גיבויים קיימים של Cobian ביעדים...
            </div>
          )}

          {askGroup && (
            <Field
              label={withGroup > 0 ? "קבוצה למשימות שאין להן קבוצה" : "קבוצה למשימות המיובאות"}
              hint={`${withGroup > 0 ? "משימות שהיו בקבוצה ב-Cobian נשארות בקבוצה שלהן. " : ""}ריק = הרשימה הכללית. אפשר להעביר משימות בין קבוצות גם אחר כך.`}
            >
              <GroupInput className="max-w-80" value={group} groups={groups} onChange={setGroup} />
            </Field>
          )}

          {!cobian && (
            <Toggle
              checked={asDisabled}
              onChange={setAsDisabled}
              label="ייבוא כמשימות מושבתות"
              description="אפשר להפעיל כל משימה אחרי שבודקים אותה"
            />
          )}

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
                      {cobian && t.group && <Badge>קבוצה: {t.group}</Badge>}
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
                    {cobian && takeOver && plans?.[t.id] && !i.error && <TakeoverSummary plan={plans[t.id]} />}
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
      )}
    </Modal>
  );
}

/** Cobian import, second step: Cobian's existing backups (if any were found) and whether to switch the tasks on. */
function OptionsStep({
  count,
  offInCobian,
  asDisabled,
  onAsDisabled,
  found,
  plans,
  takeOver,
  onTakeOver,
}: {
  count: number;
  offInCobian: number;
  asDisabled: boolean;
  onAsDisabled: (v: boolean) => void;
  /** Tasks with Cobian backups in their destination. */
  found: Task[];
  plans: Record<string, AdoptPlan> | null;
  takeOver: Takeover;
  onTakeOver: (v: Takeover) => void;
}) {
  const later = found.length > 0 && takeOver === "later";
  return (
    <div className="flex flex-col gap-5">
      {found.length > 0 && (
        <section className="flex flex-col gap-3">
          <p className="text-sm">
            {found.length === 1
              ? "ביעד של משימה אחת נמצאו גיבויים ש-Cobian כבר יצר."
              : `ביעדים של ${found.length} משימות נמצאו גיבויים ש-Cobian כבר יצר.`}{" "}
            להמשיך מהם?
          </p>
          <ChoiceCards
            value={takeOver}
            onChange={onTakeOver}
            options={[
              {
                value: "now",
                title: "להמשיך מהגיבויים הקיימים של Cobian",
                text: "מעבר חד-פעמי: התיקיות ש-Cobian יצר מקבלות את השמות של התוכנה הזו (רק שינוי שם, בלי להעתיק דבר), והגיבוי האינקרמנטלי הבא מעתיק רק מה שהשתנה מאז הגיבוי האחרון של Cobian.",
              },
              {
                value: "never",
                title: "לא להמשיך מהם",
                text: "הגיבוי הראשון של כל משימה יהיה מלא, והתיקיות של Cobian לא ייגעו.",
              },
              {
                value: "later",
                title: "רק לייבא, ולהחליט כשמפעילים את המשימה",
                text: "שום גיבוי לא מתחיל. המשימות מיובאות מושבתות, וכשתפעילו משימה התוכנה תשאל אם להמשיך מהגיבויים של Cobian.",
              },
            ]}
          />
          <ul className="flex flex-col gap-2">
            {found.map((t) => (
              <li key={t.id} className="flex flex-col gap-1.5 rounded-lg border border-line p-3">
                <span className="text-sm font-medium">{t.name}</span>
                {plans?.[t.id] && <TakeoverSummary plan={plans[t.id]} />}
              </li>
            ))}
          </ul>
        </section>
      )}

      <section className="flex flex-col gap-3">
        <p className="text-sm">
          {count === 1 ? "משימה אחת תיובא" : `${count} משימות ייובאו`}.{" "}
          {later ? "המשימות יישארו מושבתות עד שתחליטו על הגיבויים." : "להפעיל אותן כבר עכשיו?"}
        </p>
        {!later && (
          <ChoiceCards
            value={asDisabled ? "later" : "now"}
            onChange={(v) => onAsDisabled(v === "later")}
            options={[
              {
                value: "later",
                title: "להשאיר את המשימות מושבתות לבינתיים",
                text: "הן נשמרות ברשימה אבל לא ירוצו לפי התזמון. אפשר להפעיל כל משימה (או את כולן) אחרי שבודקים אותן.",
              },
              {
                value: "now",
                title: count === 1 ? "להפעיל את המשימה עכשיו" : "להפעיל את כל המשימות עכשיו",
                text:
                  offInCobian > 0
                    ? `המשימות ירוצו לפי התזמון שלהן. ${offInCobian === 1 ? "משימה אחת שהייתה מושבתת" : `${offInCobian} משימות שהיו מושבתות`} ב-Cobian נשארות מושבתות.`
                    : "המשימות ירוצו לפי התזמון שלהן.",
              },
            ]}
          />
        )}
      </section>

      <Note>
        כדי שהמשימות לא ירוצו בשתי התוכנות, השביתו אותן ב-Cobian לפני שמפעילים אותן כאן (או סגרו את Cobian).
        {found.length > 0 && takeOver === "now" && " אחרי ההמשך מהגיבויים Cobian כבר לא מזהה את התיקיות שלו ביעד."}
      </Note>

      {found.length === 0 && (
        <p className="text-[13px] text-muted">
          לא נמצאו גיבויים של Cobian ביעדים, ולכן הריצה הראשונה של כל משימה תהיה גיבוי מלא.
        </p>
      )}
    </div>
  );
}
