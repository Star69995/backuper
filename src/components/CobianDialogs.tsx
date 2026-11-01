import { AlertTriangle, ArrowLeft, ChevronDown, FileText, FolderOpen, History, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtDateTime, MODE_LABEL } from "../lib/format";
import type { AdoptPlan, AdoptResult, CobianList, CobianLists, Task } from "../types";
import { useFeedback } from "./feedback";
import { Badge, Button, cx, EmptyState, Modal, PathText, Spinner } from "./ui";

const TYPICAL_DIR = "C:\\Program Files\\Cobian Reflector\\Lists";

export const planFolderCount = (p: AdoptPlan | undefined) => p?.sources.reduce((n, s) => n + s.folders.length, 0) ?? 0;

/** Toast text for a finished takeover. */
export function takeoverToast(r: AdoptResult) {
  return r.errors.length === 0
    ? {
        tone: "ok" as const,
        title: r.renamed === 1 ? "גיבוי אחד של Cobian הומר" : `${r.renamed} גיבויים של Cobian הומרו`,
        message: "הגיבוי הבא ימשיך מהגיבוי האחרון של Cobian.",
      }
    : {
        tone: "warn" as const,
        title: `${r.renamed} גיבויים הומרו, ${r.errors.length} לא`,
        message: `${r.errors.slice(0, 3).join("; ")}. ודאו שהתיקיות לא פתוחות בתוכנה אחרת (כמו Cobian או סייר הקבצים), ונסו שוב מתפריט המשימה - "המשך מגיבויי Cobian".`,
      };
}

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
        const r = await api.takeOverCobian(ids);
        toast(takeoverToast(r));
        if (r.errors.length > 0) return setBusy(false);
      } else {
        await api.declineCobianTakeover(ids);
      }
      onResolved();
    } catch (e) {
      toast({ tone: "bad", title: takeOver ? "ההמרה נכשלה" : "העדכון נכשל", message: errorText(e) });
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
            {busy ? "ממיר..." : "המשך מגיבויי Cobian"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-[13px]">
        <p>
          {found.length === 1 ? "המשימה יובאה" : "המשימות יובאו"} מ-Cobian, וביעד {found.length === 1 ? "שלה" : "שלהן"} נמצאו
          גיבויים ש-Cobian כבר יצר. אם ממשיכים מהם, התיקיות מקבלות את השמות של התוכנה הזו (רק שינוי שם, בלי להעתיק דבר), והגיבוי
          האינקרמנטלי הבא מעתיק רק מה שהשתנה מאז הגיבוי האחרון של Cobian. בלי להמשיך, הגיבוי הראשון יהיה מלא והתיקיות של Cobian לא
          ייגעו.
        </p>
        <p className="text-muted">
          ביטול משאיר את {found.length === 1 ? "המשימה" : "המשימות"} מושבתות, ותישאלו שוב כשתפעילו{" "}
          {found.length === 1 ? "אותה" : "אותן"}. אחרי ההמשך מהגיבויים Cobian כבר לא מזהה את התיקיות - השביתו את המשימות ב-Cobian,
          כדי שלא ירוצו בשתי התוכנות.
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

/** For a saved task: finds its Cobian backups in the destination and takes them over. */
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

  const apply = async () => {
    setBusy(true);
    try {
      toast(takeoverToast(await api.takeOverCobian([task.id])));
      onDone();
    } catch (e) {
      toast({ tone: "bad", title: "ההמרה נכשלה", message: errorText(e) });
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
          <Button onClick={onClose}>{count > 0 ? "ביטול" : "סגירה"}</Button>
          {count > 0 && (
            <Button variant="primary" disabled={busy} onClick={apply}>
              {busy ? "ממיר..." : count === 1 ? "המרת גיבוי אחד" : `המרת ${count} גיבויים`}
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
          <p>
            התיקיות ש-Cobian יצר ביעד מקבלות את השמות של התוכנה הזו (רק שינוי שם, בלי להעתיק דבר). מאותו רגע הן נחשבות לגיבויים של
            המשימה: הגיבוי האינקרמנטלי הבא יעתיק רק מה שהשתנה מאז הגיבוי האחרון של Cobian, והן יימחקו לפי הגדרת השמירה של המשימה.
          </p>
          <p className="text-muted">
            אחרי ההמרה Cobian כבר לא מזהה את התיקיות האלה - השביתו את המשימה ב-Cobian, כדי שלא תרוץ בשתי התוכנות.
          </p>
          <TakeoverSummary plan={plan} defaultOpen />
        </div>
      )}
    </Modal>
  );
}
