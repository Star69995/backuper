import { ArrowLeft, Download } from "lucide-react";
import { api, errorText } from "../api";
import { fmtBytes } from "../lib/format";
import type { Snapshot } from "../types";
import { useFeedback } from "./feedback";
import { Button, Modal } from "./ui";

/** When the "auto" mode installs a found version by itself (`Updates::install_if_due`). */
export function autoInstallWhen(): string {
  return "הגרסה החדשה תותקן מעצמה כשהתוכנה לא בשימוש - החלון סגור או ממוזער, או שלא נגעו במחשב 10 דקות - ורק כשאין גיבוי שרץ או ממתין ואין שינויים שלא נשמרו.";
}

/**
 * "A new version is available": what changed, what an update does, and "update now". Opened by a
 * click on the "new version" notification (`UpdateStatus.prompt`) or on the button in the side menu.
 */
export default function UpdateDialog({ snap, onClose }: { snap: Snapshot; onClose: () => void }) {
  const { toast } = useFeedback();
  const u = snap.update;
  const mode = snap.settings.updateMode;
  const busy = u.state === "checking" || u.state === "downloading";

  const close = () => {
    if (u.prompt) api.dismissUpdatePrompt().catch(() => {});
    onClose();
  };
  const install = () => {
    if (u.prompt) api.dismissUpdatePrompt().catch(() => {});
    api.installUpdate().catch((e) => toast({ tone: "bad", title: "העדכון נכשל", message: errorText(e) }));
  };

  let status: string | null = null;
  if (u.installWaiting) status = "העדכון יותקן מיד כשהגיבוי הנוכחי (והגיבויים שבתור) יסתיימו. אין צורך לעשות דבר.";
  else if (u.state === "downloading") status = "מוריד את הגרסה החדשה...";
  else if (u.state === "ready") status = "הגרסה החדשה כבר הורדה ומוכנה להתקנה.";

  return (
    <Modal
      size="sm"
      title={
        <span className="flex items-center gap-2">
          <Download size={18} className="text-accent" />
          גרסה חדשה של Backuper
        </span>
      }
      subtitle={
        <span className="inline-flex items-center gap-1.5">
          <bdi dir="ltr">{u.currentVersion}</bdi>
          <ArrowLeft size={14} aria-label="אל" />
          <bdi dir="ltr" className="font-medium text-fg">
            {u.version}
          </bdi>
        </span>
      }
      onClose={close}
      footer={
        u.installWaiting ? (
          <Button variant="primary" onClick={close} autoFocus>
            סגירה
          </Button>
        ) : (
          <>
            <Button onClick={close}>לא עכשיו</Button>
            <Button variant="primary" icon={<Download size={15} />} disabled={busy} onClick={install} autoFocus>
              עדכון עכשיו
            </Button>
          </>
        )
      }
    >
      <div className="flex flex-col gap-3 text-sm">
        {u.notes && (
          <div className="flex flex-col gap-1">
            <span className="font-medium">מה חדש</span>
            <p dir="auto" className="max-h-48 overflow-y-auto rounded-lg border border-line bg-panel2 px-3 py-2 text-[13px] whitespace-pre-line text-muted">
              {u.notes}
            </p>
          </div>
        )}
        {status && (
          <p className="text-[13px] text-accent">
            {status}
            {u.state === "downloading" && (
              <>
                {" "}
                <bdi dir="ltr">{fmtBytes(u.downloaded)}</bdi>
                {u.total ? (
                  <>
                    {" "}
                    מתוך <bdi dir="ltr">{fmtBytes(u.total)}</bdi>
                  </>
                ) : null}
              </>
            )}
          </p>
        )}
        {u.state === "error" && u.error && <p className="text-[13px] text-bad">{u.error}</p>}
        <p className="text-[13px] text-muted">
          &quot;עדכון עכשיו&quot; סוגר את Backuper מיד לכמה שניות, והוא עולה מחדש לבד. המשימות, ההגדרות והיומן נשמרים. אם רץ גיבוי, ההתקנה
          מחכה לסיומו ולא עוצרת אותו.
        </p>
        {!u.installWaiting && (
          <p className="text-[13px] text-muted">
            {mode === "auto"
              ? autoInstallWhen()
              : "אם לא תעדכנו עכשיו, אפשר לעדכן בכל זמן מהכפתור בתחתית התפריט הצדדי או מ\"הגדרות\" - \"עדכוני תוכנה\"."}
          </p>
        )}
      </div>
    </Modal>
  );
}
