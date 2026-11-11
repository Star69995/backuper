import { AlertTriangle, ArrowLeft, ChevronDown, FileText, FolderOpen, History, Info, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtDate, fmtDateTime, MODE_LABEL } from "../lib/format";
import type { AdoptPlan, CobianList, CobianLists, CobianNaming, RestorePlan, Task } from "../types";
import { useFeedback } from "./feedback";
import { Badge, Button, cx, EmptyState, Field, Modal, PathText, Select, Spinner } from "./ui";

const TYPICAL_DIR = "C:\\Program Files\\Cobian Reflector\\Lists";

export const planFolderCount = (p: AdoptPlan | undefined) => p?.sources.reduce((n, s) => n + s.folders.length, 0) ?? 0;

/** Toast after choosing to continue from Cobian's backups (nothing is renamed yet). */
const TAKEOVER_SET = {
  tone: "ok" as const,
  title: "הגיבוי הבא ימשיך מהגיבויים של Cobian",
  message: "שמות התיקיות של Cobian ישתנו רק כשהמשימה תרוץ כאן בפעם הראשונה. עד אז Cobian ממשיך לזהות אותן.",
};

/**
 * Picks a Cobian task list: the lists found on this computer, or any file (the dialog opens
 * in Cobian's lists folder).
 */
export function CobianListsDialog({ onPick, onClose }: { onPick: (path: string, name: string) => void; onClose: () => void }) {
  const { toast } = useFeedback();
  const [found, setFound] = useState<CobianLists | null>(null);
  useEffect(() => {
    api
      .findCobianLists()
      .then(setFound)
      .catch((e) => {
        setFound({ lists: [], dir: null });
        toast({ tone: "bad", title: "החיפוש נכשל", message: errorText(e) });
      });
  }, [toast]);

  const browse = async () => {
    const path = await api.pickFile("בחירת רשימת משימות של Cobian", "רשימת משימות של Cobian", ["lst"], found?.dir ?? undefined);
    if (path) onPick(path, path.replace(/^.*[\\/]/, "").replace(/\.lst$/i, ""));
  };

  return (
    <Modal
      title="ייבוא משימות מ-Cobian"
      subtitle="רשימות המשימות של Cobian Reflector / Cobian Backup שנמצאו במחשב הזה"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>ביטול</Button>
          <Button icon={<FolderOpen size={16} />} onClick={browse} disabled={!found}>
            בחירת קובץ אחר...
          </Button>
        </>
      }
    >
      {!found ? (
        <div className="flex items-center justify-center gap-2 p-8 text-sm text-muted">
          <Spinner /> מחפש רשימות של Cobian...
        </div>
      ) : found.lists.length === 0 ? (
        <EmptyState icon={<FileText size={26} />} title="לא נמצאו רשימות של Cobian במחשב הזה">
          <p>
            Cobian Reflector שומר את רשימות המשימות בדרך כלל ב-
            <PathText path={TYPICAL_DIR} className="inline" />. אם הרשימה שמורה במקום אחר (או הועתקה ממחשב אחר), בחרו אותה עם{" "}
            <span className="font-medium">בחירת קובץ אחר</span>.
          </p>
        </EmptyState>
      ) : (
        <ul className="flex flex-col gap-2">
          {found.lists.map((l) => (
            <ListRow key={l.path} list={l} onPick={() => onPick(l.path, l.name)} />
          ))}
        </ul>
      )}
    </Modal>
  );
}

function ListRow({ list, onPick }: { list: CobianList; onPick: () => void }) {
  const ok = !list.error;
  return (
    <li>
      <button
        type="button"
        disabled={!ok}
        onClick={onPick}
        className={cx(
          "flex w-full gap-3 rounded-lg border border-line p-3 text-start text-[13px]",
          ok ? "hover:border-accent/40 hover:bg-accent-soft/50" : "cursor-not-allowed opacity-70",
        )}
      >
        <FileText size={18} className={cx("mt-0.5 shrink-0", ok ? "text-accent" : "text-muted")} />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="flex flex-wrap items-center gap-1.5">
            <bdi className="text-sm font-medium">{list.name}</bdi>
            <Badge>{list.product}</Badge>
            {list.taskCount !== null && (
              <Badge tone="accent">{list.taskCount === 1 ? "משימה אחת" : `${list.taskCount} משימות`}</Badge>
            )}
          </span>
          <PathText path={list.path} className="truncate text-muted" />
          {list.modified && <span className="text-xs text-muted">עודכנה {fmtDateTime(list.modified)}</span>}
          {list.error && (
            <span className="flex gap-1.5 text-bad">
              <XCircle size={14} className="mt-0.5 shrink-0" />
              לא ניתן לקרוא את הרשימה: {list.error}
            </span>
          )}
        </span>
      </button>
    </li>
  );
}

/** One task's takeover: how many Cobian backups were found, expandable to the full rename list. */
export function TakeoverSummary({ plan, defaultOpen = false }: { plan: AdoptPlan; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const count = planFolderCount(plan);
  const warnings = plan.sources.flatMap((s) => s.warnings);
  const leftovers = plan.sources.flatMap((s) => (s.leftovers ? [s.leftovers] : []));
  const leftCount = leftovers.reduce((n, l) => n + l.count, 0);
  const leftFirst = leftovers.map((l) => l.first).sort()[0];
  const leftLast = leftovers.map((l) => l.last).sort().at(-1);
  const multi = plan.sources.length > 1;
  return (
    <div className="flex flex-col gap-1.5 text-[13px]">
      {count > 0 ? (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setOpen(!open);
          }}
          aria-expanded={open}
          className="flex items-center gap-1.5 self-start rounded text-ok hover:underline"
        >
          <History size={14} className="shrink-0" />
          {count === 1 ? "נמצא גיבוי אחד של Cobian" : `נמצאו ${count} גיבויים של Cobian`} - הגיבוי הבא ימשיך מהם
          <ChevronDown size={14} className={cx("transition-transform", !open && "rotate-90")} />
        </button>
      ) : (
        <span className="flex items-center gap-1.5 text-muted">
          <History size={14} className="shrink-0" />
          לא נמצאו גיבויים של Cobian ביעד - הגיבוי הראשון יהיה מלא
        </span>
      )}
      {open && count > 0 && (
        <div className="flex flex-col gap-2 rounded-lg border border-line bg-panel2/60 p-2" onClick={(e) => e.stopPropagation()}>
          {plan.sources
            .filter((s) => s.folders.length > 0)
            .map((s) => (
              <div key={s.source} className="flex flex-col gap-1">
                {multi && <PathText path={s.source} className="text-xs font-medium" />}
                <ul className="flex flex-col gap-1">
                  {[...s.folders].reverse().map((f) => (
                    <li key={f.from} className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs">
                      <Badge tone={f.kind === "full" ? "accent" : "neutral"}>{MODE_LABEL[f.kind]}</Badge>
                      <bdi className="text-muted">{f.from}</bdi>
                      <ArrowLeft size={12} className="shrink-0 text-muted" />
                      <bdi>{f.to}</bdi>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
        </div>
      )}
      {leftCount > 0 && (
        <div className="flex gap-1.5 text-muted">
          <Info size={14} className="mt-0.5 shrink-0" />
          <span>
            {leftCount === 1
              ? `גיבוי אינקרמנטלי ישן אחד (${fmtDate(leftFirst)}) שייך לגיבוי מלא ש-Cobian כבר מחק - הוא נשאר ביעד כמו שהוא`
              : `${leftCount} גיבויים אינקרמנטליים ישנים (מ-${fmtDate(leftFirst)} עד ${fmtDate(leftLast)}) שייכים לגיבויים מלאים ש-Cobian כבר מחק - הם נשארים ביעד כמו שהם`}
          </span>
        </div>
      )}
      {warnings.map((w) => (
        <div key={w} className="flex gap-1.5 text-warn">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          <span>{w}</span>
        </div>
      ))}
    </div>
  );
}

/**
 * Tasks imported from Cobian that wait for the decision about Cobian's backups, and are now switched on
 * (or about to be run by hand). Nothing runs for them until it's made. Tasks with no Cobian backups in
 * their destination (any more) have nothing to decide and are released silently.
 * `onCancel` = decide later; `onResolved` = every listed task may run now.
 */
export function CobianPendingDialog({
  tasks,
  onCancel,
  onResolved,
}: {
  tasks: Task[];
  onCancel: () => void;
  onResolved: () => void;
}) {
  const { toast } = useFeedback();
  const [plans, setPlans] = useState<Record<string, AdoptPlan> | null>(null);
  const [busy, setBusy] = useState(false);
  const key = tasks.map((t) => t.id).join(",");
  useEffect(() => {
    setPlans(null);
    api
      .planCobianTakeover(tasks)
      .then((list) => setPlans(Object.fromEntries(list.map((p) => [p.taskId, p]))))
      .catch(() => setPlans({}));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  const found = plans ? tasks.filter((t) => planFolderCount(plans[t.id]) > 0) : [];
  useEffect(() => {
    if (!plans) return;
    const none = tasks.filter((t) => planFolderCount(plans[t.id]) === 0).map((t) => t.id);
    if (none.length === 0) return;
    api
      .declineCobianTakeover(none)
      .then(() => found.length === 0 && onResolved())
      .catch((e) => toast({ tone: "bad", title: "העדכון נכשל", message: errorText(e) }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plans]);
  if (!plans || found.length === 0) return null;

  const ids = found.map((t) => t.id);
  const answer = async (takeOver: boolean) => {
    setBusy(true);
    try {
      if (takeOver) {
        await api.takeOverCobian(ids);
        toast(TAKEOVER_SET);
      } else {
        await api.declineCobianTakeover(ids);
      }
      onResolved();
    } catch (e) {
      toast({ tone: "bad", title: "העדכון נכשל", message: errorText(e) });
      setBusy(false);
    }
  };

  return (
    <Modal
      title="להמשיך מהגיבויים של Cobian?"
      subtitle={found.length === 1 ? found[0].name : `${found.length} משימות`}
      size="lg"
      onClose={onCancel}
      footer={
        <>
          <Button onClick={onCancel} disabled={busy}>
            ביטול
          </Button>
          <Button disabled={busy} onClick={() => answer(false)}>
            בלי להמשיך
          </Button>
          <Button variant="primary" disabled={busy} onClick={() => answer(true)}>
            המשך מגיבויי Cobian
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-[13px]">
        <p>
          {found.length === 1 ? "המשימה יובאה" : "המשימות יובאו"} מ-Cobian, וביעד {found.length === 1 ? "שלה" : "שלהן"} נמצאו
          גיבויים ש-Cobian כבר יצר. אם ממשיכים מהם, בריצה הראשונה כאן התיקיות מקבלות את השמות של התוכנה הזו (רק שינוי שם, בלי
          להעתיק דבר), והגיבוי מעתיק רק מה שהשתנה מאז הגיבוי האחרון של Cobian. בלי להמשיך, הגיבוי הראשון יהיה מלא והתיקיות של
          Cobian לא ייגעו.
        </p>
        <p className="text-muted">
          ביטול משאיר את {found.length === 1 ? "המשימה" : "המשימות"} מושבתות, ותישאלו שוב כשתפעילו{" "}
          {found.length === 1 ? "אותה" : "אותן"}. מהריצה הראשונה כאן Cobian כבר לא מזהה את התיקיות (אפשר להחזיר לו אותן מתפריט
          המשימה) - השביתו את המשימות ב-Cobian, כדי שלא ירוצו בשתי התוכנות.
        </p>
        <ul className="flex flex-col gap-2">
          {found.map((t) => (
            <li key={t.id} className="flex flex-col gap-1.5 rounded-lg border border-line p-3">
              <span className="text-sm font-medium">{t.name}</span>
              <TakeoverSummary plan={plans[t.id]} />
            </li>
          ))}
        </ul>
      </div>
    </Modal>
  );
}

/**
 * For a saved task: finds its Cobian backups in the destination and sets the task to continue from them
 * (renamed at its next run), or cancels that while it hasn't run yet.
 */
export function CobianTakeoverDialog({ task, onClose, onDone }: { task: Task; onClose: () => void; onDone: () => void }) {
  const { toast } = useFeedback();
  const [plan, setPlan] = useState<AdoptPlan | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api
      .planCobianTakeover([task])
      .then((p) => setPlan(p[0]))
      .catch((e) => {
        toast({ tone: "bad", title: "החיפוש נכשל", message: errorText(e) });
        onClose();
      });
  }, [task, toast, onClose]);
  const count = planFolderCount(plan ?? undefined);
  const planned = task.cobianTakeover;

  const apply = async () => {
    setBusy(true);
    try {
      if (planned) {
        await api.declineCobianTakeover([task.id]);
        toast({ tone: "info", title: "ההמשך מהגיבויים של Cobian בוטל", message: "שמות התיקיות של Cobian לא ישתנו." });
      } else {
        await api.takeOverCobian([task.id]);
        toast(TAKEOVER_SET);
      }
      onDone();
    } catch (e) {
      toast({ tone: "bad", title: "העדכון נכשל", message: errorText(e) });
      setBusy(false);
    }
  };

  return (
    <Modal
      title="המשך מגיבויי Cobian"
      subtitle={task.name}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{count > 0 || planned ? "ביטול" : "סגירה"}</Button>
          {(count > 0 || planned) && (
            <Button variant={planned ? "secondary" : "primary"} disabled={busy} onClick={apply}>
              {planned ? "לא להמשיך מהם" : "להמשיך מהם בריצה הבאה"}
            </Button>
          )}
        </>
      }
    >
      {!plan ? (
        <div className="flex items-center justify-center gap-2 p-8 text-sm text-muted">
          <Spinner /> מחפש גיבויים של Cobian ב-
          <PathText path={task.destination} className="inline" />
        </div>
      ) : (
        <div className="flex flex-col gap-3 text-[13px]">
          {planned ? (
            <p>
              המשימה כבר מוגדרת להמשיך מהגיבויים של Cobian. שמות התיקיות עוד לא השתנו - הם ישתנו כשהמשימה תרוץ כאן בפעם הראשונה.
              אפשר לבטל את זה עכשיו, ואז הגיבוי הראשון יהיה מלא והתיקיות של Cobian לא ייגעו.
            </p>
          ) : (
            <p>
              כשהמשימה תרוץ כאן בפעם הבאה, התיקיות ש-Cobian יצר ביעד יקבלו את השמות של התוכנה הזו (רק שינוי שם, בלי להעתיק דבר),
              והגיבוי יעתיק רק מה שהשתנה מאז הגיבוי האחרון של Cobian. מאותו רגע הן נחשבות לגיבויים של המשימה, ויימחקו לפי הגדרת
              השמירה שלה. עד אז שום שם לא משתנה.
            </p>
          )}
          <p className="text-muted">
            מהריצה הראשונה כאן Cobian כבר לא מזהה את התיקיות האלה (אפשר להחזיר לו אותן מתפריט המשימה - "החזרת השמות של Cobian").
            השביתו את המשימה ב-Cobian, כדי שלא תרוץ בשתי התוכנות.
          </p>
          <TakeoverSummary plan={plan} defaultOpen />
        </div>
      )}
    </Modal>
  );
}

const DATE_FORMATS = [
  { value: "ymd-hms", label: "2026-10-01 03;00;00" },
  { value: "ymd-hm", label: "2026-10-01 03;00" },
  { value: "yy-hms", label: "26-10-01 03;00;00" },
  { value: "yy-hm", label: "26-10-01 03;00" },
];
const KIND_FORMATS = [
  { value: "", label: "בלי סוג" },
  { value: "word", label: "Full / Incremental" },
  { value: "paren", label: "(Full) / (Incremental)" },
];

/**
 * Gives the taken-over folders of one or more tasks Cobian's names back. Recorded renames go back
 * exactly; folders an older version took over (no record) get names rebuilt in a format the user picks.
 */
export function CobianRestoreDialog({ tasks, onClose, onDone }: { tasks: Task[]; onClose: () => void; onDone: () => void }) {
  const { toast } = useFeedback();
  const [naming, setNaming] = useState<CobianNaming>({ date: "ymd-hms", kind: "" });
  const [plans, setPlans] = useState<RestorePlan[] | null>(null);
  const [busy, setBusy] = useState(false);
  const ids = tasks.map((t) => t.id);
  const key = ids.join(",");
  useEffect(() => {
    let live = true;
    api
      .planCobianRestore(ids, naming)
      .then((p) => live && setPlans(p))
      .catch((e) => {
        toast({ tone: "bad", title: "החיפוש נכשל", message: errorText(e) });
        onClose();
      });
    return () => {
      live = false;
    };
    // Re-planned when the format changes (or the tasks); not when the parent re-renders.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, naming.date, naming.kind]);
  const all = plans?.flatMap((p) => p.folders) ?? [];
  const ready = all.filter((f) => !f.problem);
  const blocked = all.filter((f) => f.problem);
  const withFolders = plans?.filter((p) => p.folders.length > 0) ?? [];
  const multi = tasks.length > 1;
  const nameOf = (id: string) => tasks.find((t) => t.id === id)?.name ?? "";
  // Only tasks that get names back are switched off.
  const switching = plans?.filter((p) => p.folders.some((f) => !f.problem)).length ?? 0;

  const apply = async () => {
    setBusy(true);
    try {
      const r = await api.restoreCobianNames(ids, naming);
      const off = switching === 1 ? "המשימה כאן הושבתה" : "המשימות כאן הושבתו";
      toast(
        r.errors.length === 0
          ? {
              tone: "ok",
              title: r.renamed === 1 ? "השם של Cobian הוחזר לתיקייה אחת" : `השמות של Cobian הוחזרו ל-${r.renamed} תיקיות`,
              message: `Cobian מזהה אותן שוב. ${off}, כדי שלא ירוצו יחד עם Cobian.`,
            }
          : {
              tone: "warn",
              title: `${r.renamed} תיקיות קיבלו את השם של Cobian, ${r.errors.length} לא`,
              message: `${r.errors.slice(0, 3).join("; ")}. ודאו שהתיקיות לא פתוחות בתוכנה אחרת (כמו Cobian או סייר הקבצים), ונסו שוב.`,
            },
      );
      onDone();
    } catch (e) {
      toast({ tone: "bad", title: "החזרת השמות נכשלה", message: errorText(e) });
      setBusy(false);
    }
  };

  return (
    <Modal
      title="החזרת השמות של Cobian"
      subtitle={multi ? `${tasks.length} משימות` : tasks[0]?.name}
      size="lg"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{ready.length > 0 ? "ביטול" : "סגירה"}</Button>
          {ready.length > 0 && (
            <Button variant="primary" disabled={busy} onClick={apply}>
              {busy ? "משנה שמות..." : ready.length === 1 ? "החזרת השם לתיקייה אחת" : `החזרת השמות ל-${ready.length} תיקיות`}
            </Button>
          )}
        </>
      }
    >
      {!plans ? (
        <div className="flex items-center justify-center gap-2 p-8 text-sm text-muted">
          <Spinner /> מחפש את התיקיות שהומרו
          {!multi && (
            <>
              {" "}
              ב-
              <PathText path={tasks[0].destination} className="inline" />
            </>
          )}
        </div>
      ) : all.length === 0 ? (
        <EmptyState icon={<History size={26} />} title="אין תיקיות להחזיר ל-Cobian">
          <p>
            {tasks.some((t) => t.cobianTakeover)
              ? "שמות התיקיות של Cobian עוד לא השתנו - הם ישתנו רק כשהמשימה תרוץ כאן. כדי שלא ישתנו בכלל, בחרו בתפריט המשימה \"המשך מגיבויי Cobian\" ואז \"לא להמשיך מהם\"."
              : multi
                ? "לא נמצאו תיקיות של Cobian שהמשימות האלה שינו את השם שלהן."
                : "לא נמצאו תיקיות של Cobian שהמשימה הזו שינתה את השם שלהן."}
          </p>
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-3 text-[13px]">
          <p>
            התיקיות ש-Cobian יצר וקיבלו את השמות של התוכנה הזו יקבלו בחזרה את השמות של Cobian (רק שינוי שם, בלי להעתיק או למחוק דבר),
            כך ש-Cobian יזהה אותן שוב. {multi ? "כל משימה שתיקיות שלה יחזרו ל-Cobian תושבת" : "המשימה כאן תושבת"}, כדי שלא תרוץ יחד
            עם Cobian. גיבויים שהתוכנה הזו עצמה יצרה נשארים כמו שהם.
          </p>
          {plans.some((p) => p.legacy && p.folders.length > 0) && (
            <div className="flex flex-col gap-2 rounded-lg border border-warn/40 bg-warn/10 p-3">
              <p>
                {multi ? "חלק מהתיקיות הומרו" : "התיקיות האלה הומרו"} בגרסה קודמת, שלא שמרה את השמות המקוריים, ולכן השמות של Cobian
                נבנים מחדש מהתאריך. בחרו את הפורמט ש-Cobian השתמש בו - אפשר לראות אותו ביומן של Cobian, או בתיקייה ש-Cobian יצר אחרי
                המעבר. אם בשם המקורי היו שניות אחרות מ-00, תקנו את השם ידנית אחר כך.
              </p>
              <div className="grid gap-2 sm:grid-cols-2">
                <Field label="פורמט התאריך" hint="הראשון הוא ברירת המחדל של Cobian">
                  <Select dir="ltr" value={naming.date} onChange={(e) => setNaming({ ...naming, date: e.target.value })}>
                    {DATE_FORMATS.map((f) => (
                      <option key={f.value} value={f.value}>
                        {f.label}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field label="סוג הגיבוי בסוף השם">
                  <Select dir="ltr" value={naming.kind} onChange={(e) => setNaming({ ...naming, kind: e.target.value })}>
                    {KIND_FORMATS.map((f) => (
                      <option key={f.value} value={f.value}>
                        {f.label}
                      </option>
                    ))}
                  </Select>
                </Field>
              </div>
            </div>
          )}
          {withFolders.map((p) => (
            <div key={p.taskId} className="flex flex-col gap-1.5">
              {multi && <span className="text-sm font-medium">{nameOf(p.taskId)}</span>}
              <RestoreList plan={p} />
            </div>
          ))}
          {multi && withFolders.length < tasks.length && (
            <p className="text-muted">
              אין מה להחזיר ב:{" "}
              {plans
                .filter((p) => p.folders.length === 0)
                .map((p) => nameOf(p.taskId))
                .join(", ")}
              .
            </p>
          )}
          {blocked.length > 0 && ready.length > 0 && (
            <p className="text-muted">
              {blocked.length === 1 ? "תיקייה אחת לא תקבל" : `${blocked.length} תיקיות לא יקבלו`} את השם של Cobian - הסיבה מופיעה ליד
              כל אחת.
            </p>
          )}
        </div>
      )}
    </Modal>
  );
}

function RestoreList({ plan }: { plan: RestorePlan }) {
  return (
    <ul className="flex flex-col gap-1 rounded-lg border border-line bg-panel2/60 p-2">
      {[...plan.folders].reverse().map((f) => (
        <li key={f.from} className="flex flex-col gap-0.5 text-xs">
          <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5">
            <Badge tone={f.kind === "full" ? "accent" : "neutral"}>{MODE_LABEL[f.kind]}</Badge>
            <bdi className="text-muted">{f.from}</bdi>
            <ArrowLeft size={12} className="shrink-0 text-muted" />
            <bdi className={cx(f.problem && "text-muted line-through")}>{f.to}</bdi>
          </span>
          {f.problem && (
            <span className="flex gap-1.5 text-warn">
              <AlertTriangle size={12} className="mt-0.5 shrink-0" />
              {f.problem}
            </span>
          )}
        </li>
      ))}
    </ul>
  );
}
