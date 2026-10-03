import { X } from "lucide-react";
import {
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type Ref,
  type SelectHTMLAttributes,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

export const cx = (...c: (string | false | null | undefined)[]) => c.filter(Boolean).join(" ");

type Variant = "primary" | "secondary" | "ghost" | "danger";

const variants: Record<Variant, string> = {
  primary: "bg-accent text-white hover:bg-accent-hover shadow-sm",
  secondary: "bg-panel text-fg border border-line hover:bg-hover",
  ghost: "text-fg hover:bg-hover",
  danger: "bg-bad text-white hover:opacity-90 shadow-sm",
};

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: "sm" | "md"; icon?: ReactNode }) {
  return (
    <button
      type="button"
      className={cx(
        "inline-flex items-center justify-center gap-1.5 rounded-lg font-medium whitespace-nowrap transition-colors",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
        "disabled:opacity-50 disabled:pointer-events-none",
        size === "sm" ? "h-8 px-2.5 text-[13px]" : "h-9 px-3.5 text-sm",
        variants[variant],
        className,
      )}
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
}

export function IconButton({
  label,
  className,
  children,
  tone,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string; tone?: "danger" }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      className={cx(
        "inline-flex size-8 shrink-0 items-center justify-center rounded-lg transition-colors",
        "focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-40 disabled:pointer-events-none",
        tone === "danger" ? "text-bad hover:bg-bad-soft" : "text-muted hover:bg-hover hover:text-fg",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

export function Field({
  label,
  hint,
  error,
  children,
  className,
}: {
  label?: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={cx("flex flex-col gap-1.5", className)}>
      {label && <span className="text-[13px] font-medium text-fg">{label}</span>}
      {children}
      {error ? <span className="text-xs text-bad">{error}</span> : hint && <span className="text-xs text-muted">{hint}</span>}
    </label>
  );
}

/** Inputs fill their container unless the caller gives them a width class. */
const withWidth = (className?: string) => cx(!/(^|\s)(w-|max-w-|flex-1)/.test(className ?? "") && "w-full", className);

const inputCls =
  "h-9 rounded-lg border border-line bg-panel px-3 text-base text-fg placeholder:text-muted/70 " +
  "focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent/25 disabled:opacity-60";

export function TextInput({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={cx(inputCls, withWidth(className))} {...rest} />;
}

export function NumberInput({
  value,
  onChange,
  min,
  max,
  className,
  ...rest
}: Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange"> & {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
}) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  return (
    <input
      type="number"
      inputMode="numeric"
      dir="ltr"
      min={min}
      max={max}
      className={cx(inputCls, "text-center", withWidth(className))}
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        const n = Number(e.target.value);
        if (e.target.value !== "" && Number.isFinite(n)) onChange(n);
      }}
      onBlur={() => {
        let n = Number(text);
        if (!Number.isFinite(n) || text === "") n = value;
        if (min !== undefined) n = Math.max(min, n);
        if (max !== undefined) n = Math.min(max, n);
        n = Math.round(n);
        setText(String(n));
        onChange(n);
      }}
      {...rest}
    />
  );
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={cx(inputCls, "ps-3 pe-8 cursor-pointer", withWidth(className))} {...rest}>
      {children}
    </select>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
}) {
  const sw = (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cx(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:opacity-50",
        checked ? "bg-accent" : "bg-line",
      )}
    >
      <span
        className={cx(
          "absolute top-0.5 size-4 rounded-full bg-white shadow transition-[right]",
          checked ? "right-[18px]" : "right-0.5",
        )}
      />
    </button>
  );
  if (!label) return sw;
  return (
    <div className={cx("flex items-start justify-between gap-4", disabled && "opacity-60")}>
      <div className="flex flex-col gap-0.5">
        <span className="text-sm font-medium cursor-default" onClick={() => !disabled && onChange(!checked)}>
          {label}
        </span>
        {description && <span className="text-xs text-muted">{description}</span>}
      </div>
      {sw}
    </div>
  );
}

export function Checkbox({
  checked,
  indeterminate,
  onChange,
  label,
}: {
  checked: boolean;
  indeterminate?: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = !!indeterminate;
  }, [indeterminate]);
  return (
    <input
      ref={ref}
      type="checkbox"
      aria-label={label}
      checked={checked}
      onChange={(e) => onChange(e.target.checked)}
      onClick={(e) => e.stopPropagation()}
      className="size-4 cursor-pointer accent-[var(--accent)]"
    />
  );
}

type Tone = "neutral" | "accent" | "ok" | "warn" | "bad";
const tones: Record<Tone, string> = {
  neutral: "bg-panel2 text-muted border-line",
  accent: "bg-accent-soft text-accent border-transparent",
  ok: "bg-ok-soft text-ok border-transparent",
  warn: "bg-warn-soft text-warn border-transparent",
  bad: "bg-bad-soft text-bad border-transparent",
};

export function Badge({ tone = "neutral", children, icon }: { tone?: Tone; children: ReactNode; icon?: ReactNode }) {
  return (
    <span
      className={cx(
        "inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-xs font-medium whitespace-nowrap",
        tones[tone],
      )}
    >
      {icon}
      {children}
    </span>
  );
}

export function Segmented<T extends string>({
  value,
  onChange,
  options,
  className,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: ReactNode; icon?: ReactNode }[];
  className?: string;
}) {
  return (
    <div className={cx("inline-flex flex-wrap gap-1 rounded-lg border border-line bg-panel2 p-1", className)} role="radiogroup">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={cx(
            "inline-flex items-center gap-1.5 rounded-md px-3 py-1.5 text-[13px] font-medium transition-colors",
            value === o.value ? "bg-panel text-fg shadow-sm ring-1 ring-line" : "text-muted hover:text-fg",
          )}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Spinner({ className }: { className?: string }) {
  return (
    <span
      className={cx("inline-block size-4 animate-spin rounded-full border-2 border-current border-t-transparent", className)}
      aria-hidden
    />
  );
}

export function ProgressBar({ value }: { value: number | null }) {
  return (
    <div className="h-2 w-full overflow-hidden rounded-full bg-hover">
      {value === null ? (
        <div className="h-full w-2/5 rounded-full bg-accent animate-indeterminate" />
      ) : (
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-300"
          style={{ width: `${Math.min(100, Math.max(0, value))}%` }}
        />
      )}
    </div>
  );
}

export function Modal({
  title,
  subtitle,
  onClose,
  children,
  footer,
  size = "md",
  dismissable = true,
  bodyRef,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  size?: "sm" | "md" | "lg" | "xl";
  dismissable?: boolean;
  /** The scrolling body, for content that tracks its own scroll position. */
  bodyRef?: Ref<HTMLDivElement>;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const widths = { sm: "max-w-md", md: "max-w-xl", lg: "max-w-3xl", xl: "max-w-5xl" };
  return createPortal(
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-black/40 p-4 backdrop-blur-[1px]"
      onMouseDown={(e) => dismissable && e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal
        className={cx(
          "flex max-h-[min(92vh,900px)] w-full flex-col rounded-xl border border-line bg-panel shadow-2xl",
          widths[size],
        )}
      >
        <div className="flex items-start justify-between gap-4 border-b border-line px-5 py-4">
          <div className="min-w-0">
            <h2 className="text-base font-semibold">{title}</h2>
            {subtitle && <div className="mt-0.5 text-[13px] text-muted">{subtitle}</div>}
          </div>
          <IconButton label="סגור" onClick={onClose} className="-me-2 -mt-1">
            <X size={18} />
          </IconButton>
        </div>
        <div ref={bodyRef} className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {children}
        </div>
        {footer && <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line px-5 py-3">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}

export type MenuItem =
  { label: string; icon?: ReactNode; onClick: () => void; danger?: boolean; disabled?: boolean } | "separator";

/** Dropdown menu rendered in a portal (so tables with overflow don't clip it). */
export function Menu({ trigger, items }: { trigger: (open: () => void) => ReactNode; items: MenuItem[] }) {
  const anchor = useRef<HTMLSpanElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const close = () => setPos(null);

  const openMenu = () => {
    const r = anchor.current!.getBoundingClientRect();
    setPos({ top: r.bottom + 4, left: r.left });
  };

  useLayoutEffect(() => {
    if (!pos || !menu.current) return;
    const m = menu.current.getBoundingClientRect();
    const a = anchor.current!.getBoundingClientRect();
    let { top, left } = pos;
    if (top + m.height > window.innerHeight - 8) top = Math.max(8, a.top - m.height - 4);
    if (left + m.width > window.innerWidth - 8) left = Math.max(8, a.right - m.width);
    if (top !== pos.top || left !== pos.left) setPos({ top, left });
  }, [pos]);

  useEffect(() => {
    if (!pos) return;
    const onDown = (e: MouseEvent) => !menu.current?.contains(e.target as Node) && close();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [pos]);

  return (
    <>
      <span ref={anchor} className="inline-flex">
        {trigger(openMenu)}
      </span>
      {pos &&
        createPortal(
          <div
            ref={menu}
            role="menu"
            className="fixed z-50 min-w-48 rounded-lg border border-line bg-panel p-1 shadow-xl"
            style={{ top: pos.top, left: pos.left }}
          >
            {items.map((it, i) =>
              it === "separator" ? (
                <div key={i} className="my-1 h-px bg-line" />
              ) : (
                <button
                  key={i}
                  role="menuitem"
                  type="button"
                  disabled={it.disabled}
                  onClick={() => {
                    close();
                    it.onClick();
                  }}
                  className={cx(
                    "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-start text-[13px] disabled:opacity-40",
                    it.danger ? "text-bad hover:bg-bad-soft" : "hover:bg-hover",
                  )}
                >
                  <span className="flex size-4 items-center justify-center opacity-80">{it.icon}</span>
                  {it.label}
                </button>
              ),
            )}
          </div>,
          document.body,
        )}
    </>
  );
}

export function EmptyState({ icon, title, children }: { icon: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 px-6 py-16 text-center">
      <div className="flex size-14 items-center justify-center rounded-2xl bg-accent-soft text-accent">{icon}</div>
      <div className="text-base font-semibold">{title}</div>
      {children && <div className="max-w-md text-sm text-muted">{children}</div>}
    </div>
  );
}

/**
 * Paths are LTR content inside the RTL layout. Each segment is isolated so a Hebrew
 * folder name with a date ("מסמכים 2026-09-01 03-00") keeps its own order.
 */
export function PathText({ path, className, title = path }: { path: string; className?: string; title?: string }) {
  const parts = path.split(/(\\)/);
  return (
    <bdi dir="ltr" className={cx("selectable", className)} title={title}>
      {parts.map((p, i) => (p === "\\" ? p : <bdi key={i}>{p}</bdi>))}
    </bdi>
  );
}

/** A path that opens in File Explorer on click. Clicks don't reach the row underneath. */
export function PathLink({ path, onOpen, className }: { path: string; onOpen: (path: string) => void; className?: string }) {
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        onOpen(path);
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      className={cx("min-w-0 truncate rounded text-start hover:text-accent hover:underline", className)}
    >
      <PathText path={path} title={`${path}\nלחיצה לפתיחה בסייר הקבצים`} />
    </button>
  );
}

/**
 * Drag handle on a table header cell's end edge (the cell needs `relative`). Works in RTL:
 * dragging away from the cell widens it. Arrow keys resize, double-click resets.
 * Starts from the cell's rendered width, so it also works on an auto-width column;
 * `onStart` lets the table freeze its auto columns first (else they'd absorb the change).
 */
export function ColumnResizer({
  onResize,
  onReset,
  onStart,
  label,
}: {
  onResize: (w: number) => void;
  onReset: () => void;
  onStart?: () => void;
  label: string;
}) {
  const cellWidth = (el: HTMLElement) => el.parentElement!.getBoundingClientRect().width;
  const start = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget;
    el.setPointerCapture(e.pointerId);
    const rtl = getComputedStyle(el).direction === "rtl";
    const x0 = e.clientX;
    const w0 = cellWidth(el);
    onStart?.();
    const move = (ev: PointerEvent) => onResize(w0 + (rtl ? x0 - ev.clientX : ev.clientX - x0));
    const up = () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
      document.body.style.cursor = "";
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
    document.body.style.cursor = "col-resize";
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      tabIndex={0}
      title="גרירה לשינוי הרוחב, לחיצה כפולה לאיפוס"
      onPointerDown={start}
      onDoubleClick={onReset}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        const rtl = getComputedStyle(e.currentTarget).direction === "rtl";
        // The arrow pointing away from the cell widens it.
        const grow = rtl ? "ArrowLeft" : "ArrowRight";
        const shrink = rtl ? "ArrowRight" : "ArrowLeft";
        if (e.key === grow || e.key === shrink) {
          e.preventDefault();
          const w = cellWidth(e.currentTarget);
          onStart?.();
          onResize(w + (e.key === grow ? 16 : -16));
        }
      }}
      className={cx(
        "group/resize absolute inset-y-0 -end-1.5 z-10 flex w-3 cursor-col-resize touch-none items-center justify-center",
        "focus-visible:outline-none",
      )}
    >
      <span
        className={cx(
          "h-4 w-px rounded-full bg-line transition-all",
          "group-hover/resize:h-full group-hover/resize:w-0.5 group-hover/resize:bg-accent",
          "group-focus-visible/resize:h-full group-focus-visible/resize:w-0.5 group-focus-visible/resize:bg-accent",
          "group-active/resize:h-full group-active/resize:w-0.5 group-active/resize:bg-accent",
        )}
      />
    </div>
  );
}
