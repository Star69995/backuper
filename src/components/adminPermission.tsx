import { FileLock, ShieldAlert, ShieldCheck } from "lucide-react";
import { createContext, type ReactNode, useContext, useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { AdminPermission, Snapshot, Task } from "../types";
import { useFeedback } from "./feedback";
import { Button, Checkbox, cx } from "./ui";

// Permanent admin permission (elevation.rs): approved once, then open files (VSS) and stuck deletes
// run as admin without Windows's prompt - a scheduled backup has nobody there to answer it. Every
// place that may end in that prompt offers it (`AdminPermissionOffer`) until it is on.

const Ctx = createContext<{ status: AdminPermission; full: boolean; vssAll: boolean }>({ status: "unknown", full: false, vssAll: false });
/**
 * App.tsx provides `snap.adminPermission`, `snap.adminFullAccess` and `settings.vssAllTasks`, so any
 * component can read them.
 */
export const AdminPermissionProvider = Ctx.Provider;

export const NOT_ADMIN_TEXT =
  "חשבון Windows שלכם הוא משתמש רגיל, לא מנהל, ולכן אי אפשר לתת הרשאה קבועה: Windows תבקש סיסמת מנהל בכל פעם. כדי לגבות בלי לבקש, אפשר להפוך את החשבון לחשבון מנהל בהגדרות של Windows.";

/** `settings.vssAllTasks`: every task backs up open files through VSS, whatever its own switch says. */
export const useVssAll = () => useContext(Ctx).vssAll;

/** Whether a run of the task backs up open files through VSS (its own switch, or the general setting). */
export const usesVss = (t: Task, vssAll: boolean) => vssAll || t.vssOpenFiles;

/** A task that may run with nobody at the computer and may need admin then (open files through VSS). */
export const runsUnattendedWithVss = (t: Task, vssAll: boolean) =>
  t.enabled && usesVss(t, vssAll) && (t.schedule.kind !== "manual" || t.onDriveConnect === "run");

/** ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION: a file another program holds open (vss.rs `is_in_use`). */
const IN_USE = new Set([32, 33]);

/**
 * Tasks whose latest run left files another program held open and that don't back them up through VSS:
 * the permission alone doesn't make a task use VSS, the task's own option does.
 */
async function tasksMissingVss(): Promise<Task[]> {
  const [snap, history] = await Promise.all([api.snapshot(), api.history()]);
  const latest = new Map<string, (typeof history)[number]>();
  for (const r of history) {
    const seen = latest.get(r.taskId);
    if (!seen || r.finishedAt > seen.finishedAt) latest.set(r.taskId, r);
  }
  return snap.tasks.filter((t) => {
    const r = latest.get(t.id);
    return !t.vssOpenFiles && !!r && r.sources.some((s) => s.errors.some((e) => e.code != null && IN_USE.has(e.code)));
  });
}

/**
 * The choice inside the grant dialog. Full access = the admin helper never hands a request back to
 * Windows's prompt (elevation.rs); the choice is kept in an admin-only folder, so only this
 * dialog, through Windows's own prompt, changes it.
 */
function FullAccessChoice({ initial, onChange }: { initial: boolean; onChange: (v: boolean) => void }) {
  const [full, setFull] = useState(initial);
  const label = "גישה מלאה - בלי לבקש אישור אף פעם";
  return (
    <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-line p-2.5">
      <span className="pt-0.5">
        <Checkbox
          label={label}
          checked={full}
          onChange={(v) => {
            setFull(v);
            onChange(v);
          }}
        />
      </span>
      <span className="flex flex-col gap-1">
        <span className="font-medium">{label}</span>
        <span className="text-xs leading-relaxed text-muted">
          בלי הסימון, במקרים חריגים Windows עדיין תבקש אישור: יעד גיבוי שמגיעים אליו דרך קישור או כונן ממופה, או קובץ שלחשבון
          שלכם אין הרשאה לקרוא. עם הסימון גם אלה מתבצעים בלי לשאול.
        </span>
        {full && (
          <span className="text-xs leading-relaxed text-warn">
            שימו לב: עם גישה מלאה, כל תוכנה שרצה בחשבון Windows שלכם יכולה לשנות את רשימת המשימות ודרכה לקרוא כל קובץ במחשב בלי
            אישור. מתאים למחשב שרק אתם משתמשים בו.
          </span>
        )}
      </span>
    </label>
  );
}

/**
 * Asks whether to back up open files through VSS in every task (`settings.vssAllTasks`) or, when some tasks left
 * open files behind (`missing`), only in those. The permission alone doesn't make a task use VSS.
 */
export function useVssChoice() {
  const { toast, choose } = useFeedback();
  return async (missing: Task[]) => {
    const one = missing.length === 1;
    const answer = await choose<"all" | "these">({
      title: "גיבוי קבצים פתוחים דרך VSS",
      message: (
        <div className="flex flex-col gap-2">
          <p>
            קבצים שפתוחים בתוכנה אחרת (למשל קובץ הדואר של Outlook) מגובים רק במשימה שמופעל בה גיבוי דרך VSS - הרשאת המנהל רק
            מאפשרת לעשות את זה בלי לשאול. אפשר להפעיל אותו בכל המשימות בבת אחת, כולל משימות שיתווספו בהמשך (אפשר לשנות בהגדרות -
            הרשאת מנהל).
          </p>
          {missing.length > 0 && (
            <>
              <p>{one ? "בגיבוי האחרון של המשימה הזאת" : "בגיבוי האחרון של המשימות האלה"} היו קבצים פתוחים שלא הועתקו:</p>
              <ul className="list-inside list-disc">
                {missing.map((t) => (
                  <li key={t.id}>
                    <bdi>{t.name}</bdi>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      ),
      options: [
        ...(missing.length > 0 ? [{ value: "these" as const, label: one ? "רק במשימה הזאת" : "רק במשימות האלה" }] : []),
        { value: "all" as const, label: "הפעלה בכל המשימות" },
      ],
    });
    if (!answer) return false;
    try {
      // Fresh copies: the list may have changed while the dialog was open.
      const now = await api.snapshot();
      if (answer === "all") await api.saveSettings({ ...now.settings, vssAllTasks: true });
      else await api.saveTasks(now.tasks.filter((t) => missing.some((x) => x.id === t.id)).map((t) => ({ ...t, vssOpenFiles: true })));
      toast({
        tone: "ok",
        title: answer === "all" ? "הגיבוי דרך VSS הופעל בכל המשימות" : "הגיבוי דרך VSS הופעל",
        message: "קבצים פתוחים יגובו דרך VSS בכל הרצה.",
      });
      return true;
    } catch (e) {
      toast({ tone: "bad", title: "לא ניתן לשמור את השינוי", message: errorText(e) });
      return false;
    }
  };
}

/**
 * The side menu's offer of VSS, beside the permission's: while some task's latest run left open files behind and
 * doesn't use VSS. Not while the permission is off - its own offer comes first, and the grant asks about VSS.
 */
export function VssSideOffer({ snap, onDone }: { snap: Snapshot; onDone?: () => void }) {
  const { status, vssAll } = useContext(Ctx);
  const askVss = useVssChoice();
  const [missing, setMissing] = useState<Task[]>([]);
  // Recomputed when a run ends or a task's VSS switch changes.
  const key = vssAll
    ? ""
    : snap.tasks.map((t) => `${t.id}:${t.vssOpenFiles}:${snap.states[t.id]?.lastRunAt ?? ""}`).join("|");
  useEffect(() => {
    if (!key) {
      setMissing([]);
      return;
    }
    let live = true;
    tasksMissingVss().then(
      (m) => live && setMissing(m),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [key]);
  if (vssAll || missing.length === 0 || status === "off" || status === "outdated" || status === "unknown") return null;
  return (
    <button
      type="button"
      onClick={() => askVss(missing).then((done) => done && onDone?.())}
      title="גיבוי קבצים שפתוחים בתוכנה אחרת דרך VSS"
      className="flex w-full items-center gap-2 rounded-lg border border-warn/40 bg-warn-soft px-2.5 py-2 text-start text-[13px] text-warn transition-colors hover:bg-hover"
    >
      <FileLock size={16} className="shrink-0" />
      <span className="hidden flex-col leading-tight md:flex">
        <span className="font-medium text-fg">קבצים פתוחים לא גובו</span>
        <span className="text-xs">{missing.length === 1 ? "במשימה אחת" : `ב-${missing.length} משימות`} - לחצו להפעלת VSS</span>
      </span>
    </button>
  );
}

export function useAdminPermission() {
  const { status, full, vssAll } = useContext(Ctx);
  const { toast, confirm } = useFeedback();
  const askVss = useVssChoice();
  const [busy, setBusy] = useState(false);

  const grant = async () => {
    // What the dialog's checkbox says when it is confirmed (it starts from the current choice).
    let wantFull = full;
    const ok = await confirm({
      title: status === "on" ? "שינוי רמת הגישה של ההרשאה הקבועה" : "מתן הרשאת מנהל קבועה",
      message: (
        <div className="flex flex-col gap-2">
          <p>
            Windows תבקש אישור פעם אחת. אחרי זה גיבוי קבצים פתוחים (VSS) ומחיקת גיבויים ישנים שנתקעו יתבצעו בהרשאת מנהל בלי לשאול
            - גם בגיבוי מתוזמן כשאף אחד לא ליד המחשב, וגם אחרי הפעלה מחדש של המחשב.
          </p>
          <p className="text-muted">
            ההרשאה נשמרת כמשימה במתזמן המשימות של Windows, ועותק של התוכנה נשמר בתיקייה מוגנת ב-Program Files. אפשר לבטל אותה בכל
            עת בהגדרות, וגם הסרת התוכנה מבטלת אותה.
          </p>
          <FullAccessChoice initial={full} onChange={(v) => (wantFull = v)} />
        </div>
      ),
      confirmLabel: "המשך לאישור של Windows",
    });
    if (!ok) return false;
    setBusy(true);
    try {
      await api.grantAdminPermission(wantFull);
      toast({
        tone: "ok",
        title: wantFull ? "ההרשאה הקבועה הופעלה בגישה מלאה" : "ההרשאה הקבועה הופעלה",
        message: wantFull
          ? "גיבויים יבצעו פעולות מנהל בלי לבקש אישור, גם במקרים חריגים."
          : "גיבויים יבצעו פעולות מנהל בלי לבקש אישור. במקרה חריג Windows עדיין תשאל.",
      });
    } catch (e) {
      toast({ tone: "bad", title: "ההרשאה הקבועה לא הופעלה", message: errorText(e) });
      return false;
    } finally {
      setBusy(false);
    }
    await offerVss();
    return true;
  };

  /** After the grant: the permission only says how admin work runs, not that a task backs up open files. */
  const offerVss = async () => {
    if (vssAll) return;
    try {
      const [snap, missing] = await Promise.all([api.snapshot(), tasksMissingVss()]);
      if (snap.settings.vssAllTasks || snap.tasks.every((t) => t.vssOpenFiles)) return;
      await askVss(missing);
    } catch {
      /* only an offer */
    }
  };

  const revoke = async () => {
    const ok = await confirm({
      title: "ביטול ההרשאה הקבועה",
      message:
        "אחרי הביטול Windows תבקש אישור מנהל בכל פעם שגיבוי צריך אותו. בגיבוי מתוזמן שאף אחד לא מאשר, קבצים פתוחים ידולגו. גם הביטול דורש אישור של Windows.",
      confirmLabel: "ביטול ההרשאה",
      danger: true,
    });
    if (!ok) return;
    setBusy(true);
    try {
      await api.revokeAdminPermission();
      toast({ tone: "ok", title: "ההרשאה הקבועה בוטלה" });
    } catch (e) {
      toast({ tone: "bad", title: "ההרשאה לא בוטלה", message: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  return { status, full, vssAll, busy, grant, revoke };
}

/** `onDone`: after an attempt (the snapshot also refreshes by itself; the dev mock doesn't). */
export function GrantButton({ size = "sm", onDone }: { size?: "sm" | "md"; onDone?: () => void }) {
  const { status, busy, grant } = useAdminPermission();
  return (
    <Button
      className="shrink-0"
      size={size}
      variant="primary"
      icon={<ShieldCheck size={size === "sm" ? 14 : 15} />}
      disabled={busy}
      onClick={() => grant().then(() => onDone?.())}
    >
      {busy ? "ממתין לאישור של Windows..." : status === "outdated" ? "חידוש ההרשאה הקבועה" : "מתן הרשאה קבועה"}
    </Button>
  );
}

/**
 * The offer, where Windows's prompt may come up: `children` says why it matters here. Nothing when
 * the permission is on (or not known yet); a note instead of a button on a standard account.
 */
export function AdminPermissionOffer({ children, className }: { children: ReactNode; className?: string }) {
  const { status } = useAdminPermission();
  if (status === "on" || status === "unknown") return null;
  const box = "flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border p-2.5 text-[13px] leading-relaxed";
  if (status === "notAdmin") {
    return (
      <div className={cx(box, "border-line text-muted", className)}>
        <ShieldAlert size={16} className="shrink-0" />
        <span className="min-w-48 flex-1">{NOT_ADMIN_TEXT}</span>
      </div>
    );
  }
  return (
    <div className={cx(box, "border-warn/40 bg-warn-soft/50", className)}>
      <ShieldAlert size={16} className="shrink-0 text-warn" />
      <span className="min-w-48 flex-1">
        {children}
        {status === "outdated" && " ההרשאה שניתנה בגרסה קודמת צריכה אישור מחדש."}
      </span>
      <GrantButton />
    </div>
  );
}

/**
 * The side menu's standing offer: whenever the permission is off (or needs renewing), even with no
 * sign yet that a backup will need it - so a backup never waits for a "yes" nobody gives.
 */
export function AdminPermissionSideOffer({ onDone }: { onDone?: () => void }) {
  const { status, busy, grant } = useAdminPermission();
  if (status !== "off" && status !== "outdated") return null;
  return (
    <button
      type="button"
      onClick={() => grant().then(() => onDone?.())}
      disabled={busy}
      title="מתן הרשאת מנהל קבועה - כדי שגיבויים לא ייתקעו בהמתנה לאישור של Windows"
      className="flex w-full items-center gap-2 rounded-lg border border-warn/40 bg-warn-soft px-2.5 py-2 text-start text-[13px] text-warn transition-colors hover:bg-hover disabled:opacity-60"
    >
      <ShieldAlert size={16} className="shrink-0" />
      <span className="hidden flex-col leading-tight md:flex">
        <span className="font-medium text-fg">{status === "outdated" ? "חידוש הרשאת מנהל" : "הרשאת מנהל קבועה"}</span>
        <span className="text-xs">{busy ? "ממתין לאישור של Windows..." : "כדי שגיבויים לא ייתקעו - לחצו להוספה"}</span>
      </span>
    </button>
  );
}
