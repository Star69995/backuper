import { AlertTriangle, Folder, FolderOpen, FolderX, RefreshCw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtBytes, fmtDateTime, fmtRelative, MODE_LABEL } from "../lib/format";
import type { BackupFolder, SourceBackups, Task } from "../types";
import { useFeedback } from "./feedback";
import { Badge, Button, EmptyState, IconButton, Modal, PathText, Spinner } from "./ui";

export default function BackupsDialog({ task, onClose }: { task: Task; onClose: () => void }) {
  const { toast, confirm } = useFeedback();
  const [list, setList] = useState<SourceBackups[] | null>(null);
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

  const remove = async (src: SourceBackups, b: BackupFolder) => {
    const ok = await confirm({
      title: "למחוק את הגיבוי?",
      message: (
        <>
          התיקייה <bdi className="font-semibold">{b.name}</bdi> וכל הקבצים שבה יימחקו לצמיתות מהדיסק.
          {b.kind === "full" && " גיבויים אינקרמנטליים שנוצרו אחריה יישארו, אבל בלי הגיבוי המלא שלהם."}
        </>
      ),
      confirmLabel: "מחק לצמיתות",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.deleteBackup(task.id, src.folderName, b.name);
      toast({ tone: "ok", title: "הגיבוי נמחק" });
      load();
    } catch (e) {
      toast({ tone: "bad", title: "המחיקה נכשלה", message: errorText(e) });
    }
  };

  const total = list?.reduce((n, s) => n + s.backups.length, 0) ?? 0;

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
      ) : total === 0 ? (
        <EmptyState icon={<FolderX size={26} />} title="אין עדיין גיבויים">
          הגיבוי הראשון ייווצר בריצה הבאה של המשימה.
        </EmptyState>
      ) : (
        <div className="flex flex-col gap-5">
          {list.map((src) => {
            const latestFull = src.backups.find((b) => b.kind === "full" && !b.partial);
            return (
              <section key={src.folderName} className="flex flex-col gap-2">
                <h3 className="flex min-w-0 items-center gap-2 text-sm font-semibold">
                  <Folder size={16} className="shrink-0 text-accent" />
                  <bdi>{src.folderName}</bdi>
                  <span className="min-w-0 truncate text-xs font-normal text-muted">
                    <PathText path={src.source} />
                  </span>
                </h3>
                {src.backups.length === 0 ? (
                  <p className="rounded-lg border border-dashed border-line px-3 py-2 text-[13px] text-muted">
                    אין עדיין גיבויים לתיקייה זו
                  </p>
                ) : (
                  <ul className="flex flex-col gap-1.5">
                    {src.backups.map((b) => {
                      const size = sizes[b.path];
                      return (
                        <li key={b.path} className="flex flex-wrap items-center gap-3 rounded-lg border border-line px-3 py-2">
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-2 text-[13px] font-medium">
                              <bdi className="selectable">{b.name}</bdi>
                              <Badge tone={b.kind === "full" ? "accent" : "neutral"}>{MODE_LABEL[b.kind]}</Badge>
                              {b === latestFull && <Badge tone="ok">מלא אחרון</Badge>}
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
                              <bdi dir="ltr">{size === 0 ? "ריק" : fmtBytes(size)}</bdi>
                            ) : (
                              <button type="button" className="text-accent hover:underline" onClick={() => calcSize(b)}>
                                חישוב גודל
                              </button>
                            )}
                          </div>
                          <IconButton label="פתח בסייר הקבצים" onClick={() => api.openPath(b.path)}>
                            <FolderOpen size={16} />
                          </IconButton>
                          <IconButton label="מחק גיבוי" tone="danger" onClick={() => remove(src, b)}>
                            <Trash2 size={16} />
                          </IconButton>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </section>
            );
          })}
        </div>
      )}
    </Modal>
  );
}
