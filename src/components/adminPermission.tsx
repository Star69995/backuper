import { ShieldAlert, ShieldCheck } from "lucide-react";
import { createContext, type ReactNode, useContext, useState } from "react";
import { api, errorText } from "../api";
import type { AdminPermission, Task } from "../types";
import { useFeedback } from "./feedback";
import { Button, cx } from "./ui";

// Permanent admin permission (elevation.rs): approved once, then open files (VSS) and stuck deletes
// run as admin without Windows's prompt - a scheduled backup has nobody there to answer it. Every
// place that may end in that prompt offers it (`AdminPermissionOffer`) until it is on.

const Ctx = createContext<AdminPermission>("unknown");
/** App.tsx provides `snap.adminPermission`, so any component can read it. */
export const AdminPermissionProvider = Ctx.Provider;

export const NOT_ADMIN_TEXT =
  "חשבון Windows שלכם הוא משתמש רגיל, לא מנהל, ולכן אי אפשר לתת הרשאה קבועה: Windows תבקש סיסמת מנהל בכל פעם. כדי לגבות בלי לבקש, אפשר להפוך את החשבון לחשבון מנהל בהגדרות של Windows.";

/** A task that may run with nobody at the computer and may need admin then (open files through VSS). */
export const runsUnattendedWithVss = (t: Task) =>
  t.enabled && t.vssOpenFiles && (t.schedule.kind !== "manual" || t.onDriveConnect === "run");

export function useAdminPermission() {
  const status = useContext(Ctx);
  const { toast, confirm } = useFeedback();
  const [busy, setBusy] = useState(false);

  const grant = async () => {
    const ok = await confirm({
      title: "מתן הרשאת מנהל קבועה",
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
        </div>
      ),
      confirmLabel: "המשך לאישור של Windows",
    });
    if (!ok) return false;
    setBusy(true);
    try {
      await api.grantAdminPermission();
      toast({ tone: "ok", title: "ההרשאה הקבועה הופעלה", message: "גיבויים יבצעו פעולות מנהל בלי לבקש אישור." });
      return true;
    } catch (e) {
      toast({ tone: "bad", title: "ההרשאה הקבועה לא הופעלה", message: errorText(e) });
      return false;
    } finally {
      setBusy(false);
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

  return { status, busy, grant, revoke };
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
