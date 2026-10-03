import type { LucideIcon } from "lucide-react";
import { type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import { cx, PathText } from "./ui";

/** Card sections with a side nav (chips on narrow windows) and scroll spy - used by the task editor and the help page. */

export type SectionInfo<Id extends string = string> = {
  id: Id;
  title: string;
  description: string;
  icon: LucideIcon;
  /** Short current-value line under the title in the side nav. */
  summary?: string;
  /** The summary is a path (rendered LTR). */
  ltrSummary?: boolean;
  /** Something required is still missing. */
  missing?: boolean;
};

/** One card: icon + title header, then the content. */
export function Section({
  info,
  sectionRef,
  children,
}: {
  info: SectionInfo;
  sectionRef: (el: HTMLElement | null) => void;
  children: ReactNode;
}) {
  const Icon = info.icon;
  return (
    <section
      ref={sectionRef}
      aria-labelledby={`sec-${info.id}`}
      className="scroll-mt-16 overflow-hidden rounded-xl border border-line bg-panel md:scroll-mt-4"
    >
      <header className="flex items-center gap-3 border-b border-line bg-panel2 px-4 py-3">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-accent-soft text-accent">
          <Icon size={17} />
        </span>
        <div className="min-w-0">
          <h3 id={`sec-${info.id}`} className="text-sm font-semibold">
            {info.title}
          </h3>
          <p className="text-xs text-muted">{info.description}</p>
        </div>
      </header>
      <div className="flex flex-col gap-4 p-4">{children}</div>
    </section>
  );
}

function MissingDot() {
  return <span className="size-2 shrink-0 rounded-full bg-warn" title="חסר מידע" aria-label="חסר מידע" />;
}

/** Side nav on wide windows, a sticky row of chips on narrow ones. */
export function SectionNav<Id extends string>({
  sections,
  active,
  onSelect,
  label,
}: {
  sections: SectionInfo<Id>[];
  active: Id;
  onSelect: (id: Id) => void;
  label: string;
}) {
  const chips = useRef<HTMLElement>(null);
  // Keep the active chip visible. Not scrollIntoView: that would also stop the body's smooth scroll.
  useEffect(() => {
    const row = chips.current;
    const chip = row?.querySelector<HTMLElement>('[aria-current="true"]');
    if (!row || !chip || !row.offsetParent) return;
    const r = row.getBoundingClientRect();
    const c = chip.getBoundingClientRect();
    if (c.left < r.left + 16) row.scrollBy({ left: c.left - r.left - 16, behavior: "smooth" });
    else if (c.right > r.right - 16) row.scrollBy({ left: c.right - r.right + 16, behavior: "smooth" });
  }, [active]);

  return (
    <>
      <nav
        ref={chips}
        aria-label={label}
        className="sticky top-0 z-10 -mx-5 -mt-4 mb-4 flex gap-1.5 overflow-x-auto [scrollbar-width:none] border-b border-line bg-panel px-5 py-2 md:hidden"
      >
        {sections.map((s) => {
          const Icon = s.icon;
          return (
            <button
              key={s.id}
              type="button"
              aria-current={active === s.id ? "true" : undefined}
              onClick={() => onSelect(s.id)}
              className={cx(
                "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1.5 text-xs font-medium transition-colors",
                active === s.id ? "border-accent bg-accent-soft text-accent" : "border-line text-muted hover:bg-hover hover:text-fg",
              )}
            >
              <Icon size={14} />
              {s.title}
              {s.missing && <MissingDot />}
            </button>
          );
        })}
      </nav>
      <nav aria-label={label} className="sticky top-0 hidden w-48 shrink-0 md:block">
        <ul className="flex flex-col gap-0.5">
          {sections.map((s) => {
            const Icon = s.icon;
            const on = active === s.id;
            return (
              <li key={s.id}>
                <button
                  type="button"
                  aria-current={on ? "true" : undefined}
                  onClick={() => onSelect(s.id)}
                  className={cx(
                    "relative flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-start transition-colors",
                    "focus-visible:outline-2 focus-visible:outline-accent",
                    on ? "bg-accent-soft text-accent" : "hover:bg-hover",
                  )}
                >
                  {on && <span className="absolute inset-y-1.5 start-0 w-0.5 rounded-full bg-accent" />}
                  <Icon size={16} className={cx("mt-0.5 shrink-0", !on && "text-muted")} />
                  <span className="flex min-w-0 flex-1 flex-col">
                    <span className="flex items-center gap-1.5 text-[13px] font-medium">
                      {s.title}
                      {s.missing && <MissingDot />}
                    </span>
                    {s.summary && (
                      <span className="truncate text-xs text-muted" title={s.summary}>
                        {s.ltrSummary ? <PathText path={s.summary} /> : s.summary}
                      </span>
                    )}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
    </>
  );
}

/**
 * Tracks which section is in view inside the scrolling modal body. During a nav click's
 * smooth scroll the clicked section stays active, since a short last section may never
 * reach the top.
 */
export function useScrollSpy<Id extends string>(ids: Id[]) {
  const body = useRef<HTMLDivElement>(null);
  const els = useRef<Partial<Record<Id, HTMLElement | null>>>({});
  const jumping = useRef(false);
  const [active, setActive] = useState<Id>(ids[0]);
  const key = ids.join();

  useEffect(() => {
    const root = body.current;
    if (!root) return;
    const list = key.split(",") as Id[];
    const update = () => {
      if (jumping.current) return;
      const top = root.getBoundingClientRect().top;
      const atEnd = root.scrollTop + root.clientHeight >= root.scrollHeight - 2;
      let cur = list[0];
      for (const id of list) {
        const el = els.current[id];
        // 72px: below md the sticky chip row covers the top of the body.
        if (el && el.getBoundingClientRect().top - top <= 72) cur = id;
      }
      setActive(atEnd ? list[list.length - 1] : cur);
    };
    const release = () => {
      jumping.current = false;
    };
    root.addEventListener("scroll", update, { passive: true });
    root.addEventListener("scrollend", release);
    // The user scrolling by hand ends the jump right away.
    root.addEventListener("wheel", release, { passive: true });
    root.addEventListener("keydown", release);
    return () => {
      root.removeEventListener("scroll", update);
      root.removeEventListener("scrollend", release);
      root.removeEventListener("wheel", release);
      root.removeEventListener("keydown", release);
    };
  }, [key]);

  const jump = useCallback((id: Id) => {
    const el = els.current[id];
    const root = body.current;
    if (!el || !root) return;
    setActive(id);
    jumping.current = true;
    el.scrollIntoView({ behavior: "smooth", block: "start" });
    // Already in place means no scrollend; a manual scroll releases it sooner anyway.
    window.setTimeout(() => (jumping.current = false), 1000);
  }, []);

  const register = useCallback(
    (id: Id) => (el: HTMLElement | null) => {
      els.current[id] = el;
    },
    [],
  );

  return { body, active, jump, register };
}
