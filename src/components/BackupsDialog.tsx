import { AlertTriangle, FolderOpen, FolderX, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtBytes, fmtDateTime, fmtRelative } from "../lib/format";
import type { BackupFolder, Task } from "../types";
import { useFeedback } from "./feedback";
import { Badge, Button, EmptyState, IconButton, Modal, PathText, Spinner } from "./ui";

export default function BackupsDialog({ task, onClose }: { task: Task; onClose: () => void }) {
  const { toast, confirm } = useFeedback();
  const [list, setList] = useState<BackupFolder[] | null>(null);
  const [sizes, setSizes] = useState<Record<string, number | "loading">>({});

  const load = useCallback(() => {
    setList(null);
    api
      .listBackups(task.id)
      .then(setList)
      .catch((e) => {
        setList([]);
        toast({ tone: "bad", title: "טעינת הגיבויים נכשלה", message: errorText(e) });
      });
  }, [task.id, toast]);

  useEffect(load, [load]);

  const calcSize = async (b: BackupFolder) => {
    setSizes((s) => ({ ...s, [b.path]: "loading" }));
    const n = await api.folderSize(b.path);
    setSizes((s) => ({ ...s, [b.path]: n }));
  };

  const remove = async (b: BackupFolder) => {
    const ok = await confirm({
      title: "למחוק את הגיבוי?",
      message: (
        <>
          התיקייה <b>{b.name}</b> וכל הקבצים שבה יימחקו לצמיתות מהדיסק.
        </>
      ),
      confirmLabel: "מחק לצמיתות",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteBackup(task.id, b.name);
      toast({ tone: "ok", title: "הגיבוי נמחק" });
      load();
    } catch (e) {
      toast({ tone: "bad", title: "המחיקה נכשלה", message: errorText(e) });
    }
  };

  return (
    <Modal
      size="lg"
      title={`גיבויים של "${task.name}"`}
      subtitle={<PathText path={task.destination} />}
      onClose={onClose}
      footer={
        <>
          <Button icon={<RefreshCw size={15} />} onClick={load} className="me-auto">
            רענון
          </Button>
          <Button icon={<FolderOpen size={15} />} onClick={() => api.openPath(task.destination).catch(() => {})}>
            פתח את היעד
          </Button>
          <Button variant="primary" onClick={onClose}>
            סגור
          </Button>
        </>
      }
    >
      {list === null ? (
        <div className="flex justify-center py-10 text-muted">
          <Spinner />
        </div>
      ) : list.length === 0 ? (
        <EmptyState icon={<FolderX size={26} />} title="אין עדיין גיבויים">
          הגיבוי הראשון ייווצר בריצה הבאה של המשימה.
        </EmptyState>
      ) : (
        <ul className="flex flex-col gap-2">
          {list.map((b, i) => {
            const size = sizes[b.path];
            return (
              <li key={b.path} className="flex flex-wrap items-center gap-3 rounded-lg border border-line px-3 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2 font-medium">
                    <bdi className="selectable">{b.name}</bdi>
                    {i === 0 && !b.partial && <Badge tone="ok">אחרון</Badge>}
                    {b.partial && (
                      <Badge tone="warn" icon={<AlertTriangle size={12} />}>
                        לא הושלם
                      </Badge>
                    )}
                  </div>
                  <div className="mt-0.5 text-xs text-muted">
                    נוצר {fmtDateTime(b.createdAt)} ({fmtRelative(b.createdAt)})
                  </div>
                </div>
                <div className="text-[13px] text-muted">
                  {size === "loading" ? (
                    <Spinner className="size-3.5" />
                  ) : size !== undefined ? (
                    <bdi dir="ltr">{fmtBytes(size)}</bdi>
                  ) : (
                    <button type="button" className="text-accent hover:underline" onClick={() => calcSize(b)}>
                      חישוב גודל
                    </button>
                  )}
                </div>
                <IconButton label="פתח בסייר הקבצים" onClick={() => api.openPath(b.path)}>
                  <FolderOpen size={16} />
                </IconButton>
                <IconButton label="מחק גיבוי" tone="danger" onClick={() => remove(b)}>
                  <Trash2 size={16} />
                </IconButton>
              </li>
            );
          })}
        </ul>
      )}
    </Modal>
  );
}
