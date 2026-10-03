import { CalendarX, EyeOff, FileCheck, FileType, Filter, FolderX, Plus, Scale, ShieldOff, Sparkles, Trash2, Type } from "lucide-react";
import type { ReactNode } from "react";
import type { FilterKind, FilterRule } from "../types";
import { Button, IconButton, Menu, NumberInput, TextInput } from "./ui";

const KINDS: Record<FilterKind, { label: string; icon: ReactNode; hint?: string }> = {
  include: {
    label: "רק קבצים מסוג",
    icon: <FileCheck size={15} />,
    hint: "מגבים רק קבצים שתואמים לתבנית, כל השאר מדולגים. אפשר כמה, מופרדות בפסיק: ⁦*.lrcat, *.docx⁩",
  },
  extension: { label: "סיומת קובץ", icon: <FileType size={15} />, hint: "אפשר כמה, מופרדות בפסיק: tmp, log, bak" },
  pattern: { label: "שם קובץ (תבנית)", icon: <Type size={15} />, hint: "* = כל רצף תווים, ? = תו אחד. למשל ~$* או Thumbs.db" },
  folder: { label: "תיקייה", icon: <FolderX size={15} />, hint: "שם תיקייה בכל מקום בעץ (node_modules) או נתיב מלא" },
  largerThan: { label: "קבצים גדולים מ-", icon: <Scale size={15} /> },
  olderThan: { label: "קבצים ישנים מ-", icon: <CalendarX size={15} />, hint: "לפי תאריך השינוי האחרון של הקובץ" },
  hidden: { label: "קבצים מוסתרים", icon: <EyeOff size={15} /> },
  system: { label: "קבצי מערכת", icon: <ShieldOff size={15} /> },
};

const blank = (kind: FilterKind): FilterRule => {
  switch (kind) {
    case "include":
    case "extension":
    case "pattern":
    case "folder":
      return { kind, value: "" };
    case "largerThan":
      return { kind, mb: 1024 };
    case "olderThan":
      return { kind, days: 365 };
    case "hidden":
    case "system":
      return { kind };
  }
};

const PRESETS: { label: string; rules: FilterRule[] }[] = [
  {
    label: "קבצים זמניים",
    rules: [
      { kind: "extension", value: "tmp, temp, bak, part, crdownload" },
      { kind: "pattern", value: "~$*" },
    ],
  },
  {
    label: "קבצי Windows מיותרים",
    rules: [
      { kind: "pattern", value: "Thumbs.db" },
      { kind: "pattern", value: "desktop.ini" },
    ],
  },
  { label: "תיקיות פיתוח", rules: [{ kind: "folder", value: "node_modules" }] },
];

export function describeFilter(r: FilterRule) {
  switch (r.kind) {
    case "include":
    case "extension":
    case "pattern":
    case "folder":
      // Isolated left-to-right, so "*.lrcat" doesn't render as "lrcat.*" inside Hebrew text.
      return `${KINDS[r.kind].label}: ⁦${r.value}⁩`;
    case "largerThan":
      return r.mb % 1024 === 0 ? `קבצים גדולים מ-${r.mb / 1024}GB` : `קבצים גדולים מ-${r.mb}MB`;
    case "olderThan":
      return `קבצים שלא שונו ${r.days} ימים`;
    default:
      return KINDS[r.kind].label;
  }
}

const same = (a: FilterRule, b: FilterRule) => JSON.stringify(a) === JSON.stringify(b);

export default function FilterRulesEditor({
  rules,
  onChange,
  emptyText = "אין כללי סינון - כל הקבצים מגובים.",
}: {
  rules: FilterRule[];
  onChange: (r: FilterRule[]) => void;
  emptyText?: string;
}) {
  const update = (i: number, r: FilterRule) => onChange(rules.map((x, j) => (j === i ? r : x)));
  const remove = (i: number) => onChange(rules.filter((_, j) => j !== i));
  const add = (r: FilterRule[]) => onChange([...rules, ...r.filter((n) => !rules.some((x) => same(x, n)))]);
  const singleton = (k: FilterKind) => (k === "hidden" || k === "system") && rules.some((r) => r.kind === k);

  return (
    <div className="flex flex-col gap-2">
      {rules.length === 0 ? (
        <div className="flex items-center gap-2 rounded-lg border border-dashed border-line px-3 py-3 text-[13px] text-muted">
          <Filter size={15} />
          {emptyText}
        </div>
      ) : (
        <ul className="flex flex-col gap-2">
          {rules.map((r, i) => {
            const k = KINDS[r.kind];
            return (
              <li key={i} className="flex flex-wrap items-start gap-2 rounded-lg border border-line bg-panel2 px-3 py-2">
                <span className="flex h-9 w-40 shrink-0 items-center gap-2 text-[13px] font-medium">
                  <span className="text-muted">{k.icon}</span>
                  {k.label}
                </span>
                <div className="flex min-w-48 flex-1 flex-col gap-1">
                  {(r.kind === "include" || r.kind === "extension" || r.kind === "pattern" || r.kind === "folder") && (
                    <TextInput
                      dir="ltr"
                      className="text-left font-mono"
                      autoFocus={!r.value}
                      value={r.value}
                      placeholder={r.kind === "include" ? "*.lrcat" : r.kind === "extension" ? "tmp, log" : r.kind === "pattern" ? "~$*" : "node_modules"}
                      onChange={(e) => update(i, { ...r, value: e.target.value })}
                    />
                  )}
                  {r.kind === "largerThan" && (
                    <div className="flex items-center gap-2">
                      <NumberInput className="w-28" min={1} value={r.mb} onChange={(mb) => update(i, { ...r, mb })} />
                      <span className="text-[13px] text-muted">MB</span>
                    </div>
                  )}
                  {r.kind === "olderThan" && (
                    <div className="flex items-center gap-2">
                      <NumberInput className="w-28" min={1} value={r.days} onChange={(days) => update(i, { ...r, days })} />
                      <span className="text-[13px] text-muted">ימים</span>
                    </div>
                  )}
                  {k.hint && <span className="text-xs text-muted">{k.hint}</span>}
                </div>
                <IconButton label="הסרת כלל" tone="danger" onClick={() => remove(i)} className="mt-0.5">
                  <Trash2 size={15} />
                </IconButton>
              </li>
            );
          })}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Menu
          trigger={(open) => (
            <Button size="sm" icon={<Plus size={15} />} onClick={open}>
              הוספת כלל
            </Button>
          )}
          items={(Object.keys(KINDS) as FilterKind[]).map((kind) => ({
            label: KINDS[kind].label.replace(/-$/, ""),
            icon: KINDS[kind].icon,
            disabled: singleton(kind),
            onClick: () => add([blank(kind)]),
          }))}
        />
        <span className="flex items-center gap-1 text-xs text-muted">
          <Sparkles size={13} /> הוספה מהירה:
        </span>
        {PRESETS.map((p) => (
          <button
            key={p.label}
            type="button"
            onClick={() => add(p.rules)}
            className="rounded-full border border-line px-2.5 py-1 text-xs hover:bg-hover"
          >
            {p.label}
          </button>
        ))}
      </div>
    </div>
  );
}
