import { AlertTriangle, FolderOpen } from "lucide-react";
import { api, errorText } from "../api";
import type { Snapshot } from "../types";
import { useFeedback } from "./feedback";
import { Button, Modal } from "./ui";

/** A message from the backend that waits until the user dismisses it (e.g. the task list was recovered). */
export default function NoticeDialog({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast } = useFeedback();
  const notice = snap.notice;
  if (!notice) return null;

  const dismiss = () =>
    api
      .dismissNotice()
      .then(refresh)
      .catch((e) => toast({ tone: "bad", title: "הפעולה נכשלה", message: errorText(e) }));

  return (
    <Modal
      size="sm"
      title={
        <span className="flex items-center gap-2">
          <AlertTriangle size={18} className="text-warn" />
          {notice.title}
        </span>
      }
      onClose={dismiss}
      footer={
        <>
          {notice.path && (
            <Button icon={<FolderOpen size={16} />} onClick={() => api.openPath(notice.path!)}>
              פתיחת התיקייה
            </Button>
          )}
          <Button variant="primary" onClick={dismiss}>
            הבנתי
          </Button>
        </>
      }
    >
      <p className="text-sm">{notice.message}</p>
    </Modal>
  );
}
