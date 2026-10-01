import { AlertTriangle, CheckCircle2, Info, X, XCircle } from "lucide-react";
import { createContext, type ReactNode, useCallback, useContext, useRef, useState } from "react";
import { Button, cx, Modal } from "./ui";

type ToastTone = "ok" | "bad" | "warn" | "info";
interface Toast {
  id: number;
  tone: ToastTone;
  title: string;
  message?: string;
  action?: { label: string; onClick: () => void };
}
interface ConfirmOpts {
  title: string;
  message?: ReactNode;
  confirmLabel?: string;
  danger?: boolean;
}

interface Feedback {
  toast: (t: Omit<Toast, "id">) => void;
  confirm: (o: ConfirmOpts) => Promise<boolean>;
}

const Ctx = createContext<Feedback>(null!);
export const useFeedback = () => useContext(Ctx);

const toneIcon: Record<ToastTone, ReactNode> = {
  ok: <CheckCircle2 size={18} className="text-ok" />,
  bad: <XCircle size={18} className="text-bad" />,
  warn: <AlertTriangle size={18} className="text-warn" />,
  info: <Info size={18} className="text-accent" />,
};

export function FeedbackProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [confirmState, setConfirm] = useState<(ConfirmOpts & { resolve: (v: boolean) => void }) | null>(null);
  const nextId = useRef(1);

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const toast = useCallback(
    (t: Omit<Toast, "id">) => {
      const id = nextId.current++;
      setToasts((list) => [...list.slice(-3), { ...t, id }]);
      setTimeout(() => dismiss(id), t.action ? 9000 : 5000);
    },
    [dismiss],
  );

  const confirm = useCallback((o: ConfirmOpts) => new Promise<boolean>((resolve) => setConfirm({ ...o, resolve })), []);

  const finish = (v: boolean) => {
    confirmState?.resolve(v);
    setConfirm(null);
  };

  return (
    <Ctx.Provider value={{ toast, confirm }}>
      {children}
      <div className="pointer-events-none fixed bottom-4 left-4 z-[60] flex w-[min(380px,calc(100vw-32px))] flex-col gap-2">
        {toasts.map((t) => (
          <div
            key={t.id}
            className="pointer-events-auto flex items-start gap-3 rounded-xl border border-line bg-panel p-3 shadow-xl"
          >
            <span className="mt-0.5">{toneIcon[t.tone]}</span>
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium">{t.title}</div>
              {t.message && <div className="mt-0.5 text-[13px] break-words text-muted">{t.message}</div>}
            </div>
            {t.action && (
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  t.action!.onClick();
                  dismiss(t.id);
                }}
              >
                {t.action.label}
              </Button>
            )}
            <button type="button" aria-label="סגור" onClick={() => dismiss(t.id)} className="text-muted hover:text-fg">
              <X size={16} />
            </button>
          </div>
        ))}
      </div>
      {confirmState && (
        <Modal
          size="sm"
          title={confirmState.title}
          onClose={() => finish(false)}
          footer={
            <>
              <Button onClick={() => finish(false)}>ביטול</Button>
              <Button variant={confirmState.danger ? "danger" : "primary"} onClick={() => finish(true)} autoFocus>
                {confirmState.confirmLabel ?? "אישור"}
              </Button>
            </>
          }
        >
          <div className={cx("text-sm leading-relaxed")}>{confirmState.message}</div>
        </Modal>
      )}
    </Ctx.Provider>
  );
}
