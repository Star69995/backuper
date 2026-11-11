import {
  AlertTriangle,
  CalendarX,
  Check,
  ArrowDownUp,
  CircleHelp,
  Copy,
  Download,
  EyeOff,
  File,
  FileType,
  FileX,
  Filter,
  Folder,
  FolderX,
  Info,
  Plus,
  Regex,
  Scale,
  ShieldOff,
  Sparkles,
  TestTube,
  Trash2,
  Type,
  Upload,
  X,
} from "lucide-react";
import { useEffect, useId, useMemo, useState, type ReactNode } from "react";
import { api, errorText } from "../api";
import { addRules, relations, tidy, type Added, type Relation } from "../lib/filterRules";
import { CACHE_FOLDERS } from "../lib/cacheFolders";
import type { FilterCheck, FilterKind, FilterProbe, FilterRule, RegexCheck, RuleResult } from "../types";
import { useFeedback } from "./feedback";
import { AutoGrowInput, Badge, Button, Checkbox, IconButton, Menu, NumberInput, Segmented, TextInput } from "./ui";

const KINDS: Record<FilterKind, { label: string; icon: ReactNode; hint?: string }> = {
  // The three file-name kinds share one row (see isFiles); "include" is its "only these" mode.
  include: { label: "סוגי קבצים ושמות", icon: <FileType size={15} /> },
  includeName: { label: "סוגי קבצים ושמות", icon: <Type size={15} /> },
  extension: { label: "סוגי קבצים ושמות", icon: <FileType size={15} /> },
  pattern: { label: "סוגי קבצים ושמות", icon: <Type size={15} /> },
  regex: {
    label: "ביטוי רגולרי (מתקדם)",
    icon: <Regex size={15} />,
    hint: "הביטוי נבדק מול שם הקובץ בלבד, בלי הבחנה בין אותיות גדולות לקטנות. למשל ⁦^IMG_\\d+⁩ - שמות שמתחילים ב-IMG_ ואחריו ספרות.",
  },
  folder: {
    label: "תיקייה",
    icon: <FolderX size={15} />,
    hint: "התיקייה ומה שבתוכה לא יגובו. שם תיקייה בכל מקום בעץ (node_modules), נתיב מלא או נתיב יחסי לתיקיית המקור (Projects\\cache). אפשר * (כל טקסט) ו-? (תו אחד), ונתיב שמתחיל ב-* חל בכל עומק: *\\Users\\User.",
  },
  file: {
    label: "קובץ מסוים",
    icon: <FileX size={15} />,
    hint: "הקובץ הזה לא יגובה. נתיב מלא, למשל C:\\pagefile.sys, או נתיב יחסי לתיקיית המקור, למשל Docs\\a.txt (בלי * או ?; לשמות לפי תבנית יש את הכלל של סוגי קבצים ושמות). נוח לקובץ שנכשל בכל גיבוי כי הוא תמיד נעול.",
  },
  largerThan: { label: "קבצים גדולים", icon: <Scale size={15} />, hint: "קבצים גדולים מהגודל הזה לא יגובו." },
  olderThan: {
    label: "קבצים ישנים",
    icon: <CalendarX size={15} />,
    hint: "קבצים שלא שונו מעל מספר הימים הזה לא יגובו (לפי תאריך השינוי האחרון של הקובץ).",
  },
  hidden: { label: "קבצים מוסתרים", icon: <EyeOff size={15} />, hint: "קבצים עם התכונה \"מוסתר\" של Windows לא יגובו." },
  system: {
    label: "קבצי מערכת",
    icon: <ShieldOff size={15} />,
    hint: "קבצים עם התכונה \"מערכת\" של Windows לא יגובו, בלי קשר לשם או למיקום שלהם. אלה קבצים ש-Windows מסמנת לעצמה, כמו pagefile.sys, hiberfil.sys, ntuser.dat ו-desktop.ini. מתאים בעיקר לגיבוי של כונן שלם או של תיקיית משתמש. תיקיות לא מושפעות, וגם לא קבצים שרק מוסתרים (יש לזה כלל נפרד).",
  },
};

/** Rules offered in the add menu. */
const MENU: FilterKind[] = ["extension", "regex", "folder", "file", "largerThan", "olderThan", "hidden", "system"];

type FilesKind = "include" | "includeName" | "extension" | "pattern";

const isFiles = (r: FilterRule): r is Extract<FilterRule, { kind: FilesKind }> =>
  r.kind === "include" || r.kind === "includeName" || r.kind === "extension" || r.kind === "pattern";

/** "Only these" (back up only matches) vs "skip these". */
const isOnly = (r: FilterRule) => r.kind === "include" || r.kind === "includeName" || (r.kind === "regex" && r.include);

/** A file-name rule that matches whole names ("Thumbs.db", "~$*") rather than extensions ("tmp"). */
const isByName = (r: FilterRule) => r.kind === "includeName" || r.kind === "pattern";

const filesKind = (only: boolean, byName: boolean): FilesKind =>
  only ? (byName ? "includeName" : "include") : byName ? "pattern" : "extension";

const splitList = (v: string) =>
  v
    .split(/[,;]/)
    .map((x) => x.trim())
    .filter(Boolean);

/** Extensions -> names when the rule is switched to "names": "tmp, .log" -> "*.tmp, *.log" (what they matched). */
const extsToNames = (v: string) =>
  splitList(v)
    .map((x) => (/[*?]/.test(x) ? x : x.startsWith(".") ? `*${x}` : x.includes(".") ? x : `*.${x}`))
    .join(", ");

/** Names -> extensions when switched back: only the plain "*.ext" entries become bare extensions. */
const namesToExts = (v: string) =>
  splitList(v)
    .map((x) => x.match(/^\*\.([^.*?]+)$/)?.[1] ?? x)
    .join(", ");

const FILES_HINT = {
  skipExt: "קבצים עם הסיומות האלה לא יגובו. מפרידים בפסיק, למשל tmp, log, bak.",
  onlyExt: "רק קבצים עם הסיומות האלה יגובו, כל השאר לא. מפרידים בפסיק, למשל docx, xlsx.",
  skipName: "קבצים עם השמות האלה לא יגובו. שם מלא כולל סיומת, מפרידים בפסיק, למשל Thumbs.db, desktop.ini. הסימן * מייצג כל טקסט: ~$*",
  onlyName: "רק קבצים עם השמות האלה יגובו, כל השאר לא. שם מלא כולל סיומת, מפרידים בפסיק, למשל notes.txt, report*. הסימן * מייצג כל טקסט.",
};

const REGEX_FOLDERS_HINT =
  "הביטוי נבדק מול שם התיקייה בלבד, בכל עומק בעץ, בלי הבחנה בין אותיות גדולות לקטנות. תיקייה תואמת ומה שבתוכה לא יגובו. למשל ⁦^(build|cache)\\d*$⁩ - build, cache2 ועוד. אפשר רק לדלג על תיקיות, לא לגבות רק אותן.";

const PLACEHOLDERS: Partial<Record<FilterKind, string>> = {
  include: "docx, xlsx, jpg",
  includeName: "notes.txt, report*",
  extension: "tmp, log, bak",
  pattern: "Thumbs.db, ~$*",
  regex: "^IMG_\\d+",
  folder: "node_modules",
  file: "C:\\pagefile.sys",
};

/** The explanation of a rule, shown on demand (and right away while the rule is still empty). */
const hintOf = (r: FilterRule) => {
  if (isFiles(r)) return FILES_HINT[`${isOnly(r) ? "only" : "skip"}${isByName(r) ? "Name" : "Ext"}`];
  if (r.kind === "regex" && r.folders) return REGEX_FOLDERS_HINT;
  return KINDS[r.kind].hint;
};

const hasText = (r: FilterRule): r is Extract<FilterRule, { value: string }> => "value" in r;

const blank = (kind: FilterKind): FilterRule => {
  switch (kind) {
    case "include":
    case "includeName":
    case "extension":
    case "pattern":
    case "folder":
    case "file":
      return { kind, value: "" };
    case "regex":
      return { kind, value: "", include: false, folders: false };
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
  {
    label: "קבצי מטמון",
    rules: CACHE_FOLDERS.map((value) => ({ kind: "folder", value })),
  },
  {
    label: "תיקיות פיתוח",
    rules: [
      { kind: "folder", value: "node_modules" },
      { kind: "folder", value: ".venv" },
      { kind: "folder", value: "dist" },
    ],
  },
];

export function describeFilter(r: FilterRule) {
  switch (r.kind) {
    // Values are isolated left-to-right, so "*.lrcat" doesn't render as "lrcat.*" inside Hebrew text.
    case "include":
    case "includeName":
    case "extension":
    case "pattern":
      return `${isOnly(r) ? "לגבות רק" : "לדלג על"}: ⁦${r.value}⁩`;
    case "regex":
      return r.folders
        ? `לדלג על תיקיות לפי ביטוי רגולרי: ⁦${r.value}⁩`
        : `${r.include ? "לגבות רק" : "לדלג על"} ביטוי רגולרי: ⁦${r.value}⁩`;
    case "folder":
      return `תיקייה: ⁦${r.value}⁩`;
    case "file":
      return `קובץ: ⁦${r.value}⁩`;
    case "largerThan":
      return r.mb % 1024 === 0 ? `קבצים גדולים מ-${r.mb / 1024}GB` : `קבצים גדולים מ-${r.mb}MB`;
    case "olderThan":
      return `קבצים שלא שונו ${r.days} ימים`;
    default:
      return KINDS[r.kind].label;
  }
}

/**
 * Under a regex rule: whether it is valid (and why not), what it says piece by piece, and a try-out
 * field. Parsing and wording come from the backend, so the reading is exactly what the engine does.
 */
function RegexCheckPanel({
  rule,
  sample,
  setSample,
}: {
  rule: Extract<FilterRule, { kind: "regex" }>;
  sample: string;
  setSample: (s: string) => void;
}) {
  const id = useId();
  const [check, setCheck] = useState<RegexCheck | null>(null);
  const pattern = rule.value;
  const subject = rule.folders ? "תיקייה" : "קובץ";

  useEffect(() => {
    if (!pattern.trim()) {
      setCheck(null);
      return;
    }
    let live = true;
    const t = setTimeout(() => {
      api
        .checkRegex(pattern, sample || null)
        .then((c) => live && setCheck(c))
        .catch(() => live && setCheck(null));
    }, 200);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [pattern, sample]);

  if (!check) return null;
  if (check.error)
    return (
      <div className="mt-1.5 flex items-start gap-1.5 rounded-md bg-bad-soft px-2.5 py-1.5 text-xs text-bad">
        <AlertTriangle size={14} className="mt-0.5 shrink-0" />
        <span>הביטוי לא תקין: {check.error}</span>
      </div>
    );

  const skips = rule.folders || !rule.include;
  // What the rule does with the try-out name: a skip rule drops what matches, an "only" rule keeps only that.
  const kept = check.matched === !skips;
  return (
    <div className="mt-1.5 flex flex-col gap-2 rounded-md border border-line bg-panel px-2.5 py-2 text-xs">
      <div className="font-medium text-muted">מה הביטוי אומר, לפי הסדר</div>
      <ul className="flex flex-col gap-1">
        {check.parts.map((p, i) => (
          <li key={i} className="flex items-baseline gap-2">
            <code dir="ltr" className="shrink-0 rounded bg-panel2 px-1.5 py-0.5 font-mono text-[13px]">
              {p.token}
            </code>
            <span>{p.meaning}</span>
          </li>
        ))}
      </ul>
      {check.note && (
        <div className="flex items-start gap-1.5 text-muted">
          <Info size={13} className="mt-0.5 shrink-0" />
          <span>{check.note}</span>
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2 border-t border-line pt-2">
        <label htmlFor={id} className="text-muted">
          בדיקה על שם {subject}:
        </label>
        <TextInput
          id={id}
          dir="ltr"
          className="min-w-40 flex-1 text-left font-mono"
          placeholder={rule.folders ? "build2" : ".gitignore"}
          value={sample}
          onChange={(e) => setSample(e.target.value)}
        />
        {check.matched !== null && (
          <>
            <Badge tone={check.matched ? "accent" : "neutral"}>{check.matched ? "מתאים לביטוי" : "לא מתאים לביטוי"}</Badge>
            <Badge tone={kept ? "ok" : "warn"}>{kept ? `${subject} יגובה` : `${subject} לא יגובה`}</Badge>
            {check.matched && check.matchedText && check.matchedText !== sample && (
              <span className="text-muted">
                החלק שהתאים: <bdi dir="ltr">{check.matchedText}</bdi>
              </span>
            )}
          </>
        )}
      </div>
    </div>
  );
}

const EFFECT: Record<
  RuleResult["effect"],
  { label: string; tone: "neutral" | "accent" | "ok" | "warn" | "bad"; icon: ReactNode }
> = {
  none: { label: "לא משפיע", tone: "neutral", icon: null },
  skips: { label: "מדלג", tone: "warn", icon: <X size={12} /> },
  keeps: { label: "תואם", tone: "ok", icon: <Check size={12} /> },
  unknown: { label: "לא נבדק", tone: "neutral", icon: <CircleHelp size={12} /> },
  invalid: { label: "לא שלם", tone: "bad", icon: <AlertTriangle size={12} /> },
};

/** The rule's part in the filter check, on its row. "No effect" stays quiet. */
function EffectBadge({ result }: { result: RuleResult }) {
  if (result.effect === "none") return null;
  const e = EFFECT[result.effect];
  return (
    <span title={result.detail ?? undefined}>
      <Badge tone={e.tone} icon={e.icon}>
        {e.label}
      </Badge>
    </span>
  );
}

const fmtSize = (b: number) =>
  b >= 1024 ** 3
    ? `${(b / 1024 ** 3).toFixed(1)} GB`
    : b >= 1024 ** 2
      ? `${(b / 1024 ** 2).toFixed(1)} MB`
      : b >= 1024
        ? `${Math.round(b / 1024)} KB`
        : `${b} B`;

/** Optional number field: empty = not given. */
function OptionalNumber({
  value,
  onChange,
  label,
  unit,
}: {
  value: number | null;
  onChange: (v: number | null) => void;
  label: string;
  unit: string;
}) {
  const id = useId();
  return (
    <span className="flex items-center gap-1.5">
      <label htmlFor={id} className="text-muted">
        {label}
      </label>
      <TextInput
        id={id}
        dir="ltr"
        inputMode="decimal"
        className="w-24 text-center"
        value={value ?? ""}
        onChange={(e) => {
          const n = Number(e.target.value);
          onChange(e.target.value.trim() === "" || !Number.isFinite(n) || n < 0 ? null : n);
        }}
      />
      <span className="text-muted">{unit}</span>
    </span>
  );
}

/** Skip rules that would leave out the path of a check, from the most exact to the widest. */
function skipRulesFor(check: FilterCheck): { label: string; rule: FilterRule }[] {
  const full = check.fullPath?.replace(/\\+$/, "");
  if (!full) return [];
  const name = full.slice(full.lastIndexOf("\\") + 1);
  const out: { label: string; rule: FilterRule }[] = [];
  if (check.folder) {
    out.push({ label: "התיקייה הזו", rule: { kind: "folder", value: full } });
    out.push({ label: "כל תיקייה בשם הזה", rule: { kind: "folder", value: name } });
    return out;
  }
  out.push({ label: "הקובץ הזה", rule: { kind: "file", value: full } });
  // A name goes into a comma-separated list, so one with a comma can't be a name rule.
  if (!/[,;]/.test(name)) out.push({ label: "כל קובץ בשם הזה", rule: { kind: "pattern", value: name } });
  const ext = name.slice(name.lastIndexOf(".") + 1);
  if (name.lastIndexOf(".") > 0 && /^[^\s,;.*?]+$/.test(ext))
    out.push({ label: `כל קובץ ${ext.toLowerCase()}`, rule: { kind: "extension", value: ext } });
  return out;
}

/** One reading of the checked path (as a file or as a folder): the verdict, which rules decide it, and skip buttons. */
function CheckResult({
  c,
  title,
  rules,
  globalRules,
  onAdd,
  first,
}: {
  c: FilterCheck;
  title?: string;
  rules: FilterRule[];
  globalRules: FilterRule[];
  onAdd: (r: FilterRule[]) => void;
  /** The first (or only) reading carries the divider and the "no rules at all" line. */
  first?: boolean;
}) {
  const subject = c.folder ? "התיקייה" : "הקובץ";
  const affecting = [
    ...rules.map((r, i) => ({ rule: r, result: c.rules[i], global: false })),
    ...globalRules.map((r, i) => ({ rule: r, result: c.globalRules[i], global: true })),
  ].filter((x) => x.result && x.result.effect !== "none");
  const quiet = rules.length + globalRules.length - affecting.length;
  const skipOptions = c.kept ? skipRulesFor(c) : [];
  return (
    <div className={`flex flex-col gap-1.5 ${c.asFolder ? "" : "border-t border-line pt-2"}`}>
      {title && <div className="text-[13px] font-medium">{title}</div>}
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={c.kept ? "ok" : "warn"} icon={c.kept ? <Check size={12} /> : <X size={12} />}>
          {c.kept ? `${subject} יגובה` : `${subject} לא יגובה`}
        </Badge>
        {c.reason && <span>{c.reason}</span>}
      </div>
      {c.onDisk && (
        <div className="text-muted">
          נבדק מול {subject} שבדיסק
          {!c.folder && (
            <>
              : <bdi dir="ltr">{fmtSize(c.onDisk.size)}</bdi>
              {c.onDisk.modified && `, שונה ב-${new Date(c.onDisk.modified).toLocaleDateString("he-IL")}`}
              {c.onDisk.hidden && ", מוסתר"}
              {c.onDisk.system && ", קובץ מערכת"}
            </>
          )}
        </div>
      )}
      {affecting.length > 0 && (
        <ul className="flex flex-col gap-1">
          {affecting.map(({ rule, result, global }, i) => (
            <li key={i} className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
              <EffectBadge result={result!} />
              <span>{describeFilter(rule)}</span>
              {global && <span className="text-muted">(כלל כללי)</span>}
              {result!.detail && <span className="text-muted">- {result!.detail}</span>}
            </li>
          ))}
        </ul>
      )}
      {quiet > 0 && affecting.length > 0 && (
        <div className="text-muted">
          {quiet === 1 ? "כלל אחד נוסף לא משפיע" : `${quiet} כללים נוספים לא משפיעים`} על {subject} הזה.
        </div>
      )}
      {first && rules.length + globalRules.length === 0 && <div className="text-muted">אין כללי סינון, אז כל הקבצים מגובים.</div>}
      {skipOptions.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 pt-0.5">
          <span className="text-muted">לא רוצים לגבות {c.folder ? "אותה" : "אותו"}? הוספת כלל לדילוג על:</span>
          {skipOptions.map((o) => (
            <Button key={o.label} size="sm" icon={KINDS[o.rule.kind].icon} onClick={() => onAdd([o.rule])} title={describeFilter(o.rule)}>
              {o.label}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * "What happens to this file?": one path run through all the rules together (the task's and the
 * global ones), with the engine's own matching. Each rule's part shows on its row and in the list here.
 */
function FilterCheckPanel({
  rules,
  globalRules,
  sources,
  probe,
  setProbe,
  check,
  onAdd,
  onClose,
}: {
  rules: FilterRule[];
  globalRules: FilterRule[];
  sources: string[];
  probe: FilterProbe;
  setProbe: (p: FilterProbe) => void;
  check: FilterCheck | null;
  /** Adds skip rules to the task's rules. */
  onAdd: (r: FilterRule[]) => void;
  onClose: () => void;
}) {
  const id = useId();
  const any = (k: FilterKind) => rules.some((r) => r.kind === k) || globalRules.some((r) => r.kind === k);
  // Size, date and attributes are asked only for a file that isn't on disk, and only when a rule looks at them.
  const askValues = check?.kept != null && !check.onDisk && !check.folder;
  const pick = async (folder: boolean) => {
    const p = folder
      ? await api.pickFolder("בחירת תיקייה לבדיקה", sources[0])
      : await api.pickFile("בחירת קובץ לבדיקה", "כל הקבצים", ["*"], sources[0]);
    if (p) setProbe({ ...probe, path: folder ? `${p.replace(/\\+$/, "")}\\` : p });
  };

  return (
    <div className="flex flex-col gap-2 rounded-lg border border-accent/30 bg-panel px-3 py-2.5 text-xs">
      <div className="flex items-center gap-2">
        <TestTube size={15} className="text-accent" />
        <span className="flex-1 text-[13px] font-medium">בדיקת כל הכללים יחד</span>
        <IconButton label="סגירת הבדיקה" onClick={onClose}>
          <X size={15} />
        </IconButton>
      </div>
      <p className="text-muted">
        הקלידו נתיב של קובץ או תיקייה {sources.length > 0 ? "(מלא, או בתוך תיקיית המקור)" : "(מלא, או רק שם קובץ)"} ותראו אם הוא יגובה
        ואיזה כלל מחליט. כל הכללים נבדקים יחד{globalRules.length > 0 ? ", כולל הכלליים" : ""}, כמו בגיבוי עצמו. קובץ שקיים בדיסק
        נבדק עם הגודל, התאריך והתכונות שלו. לבדיקת תיקייה מסיימים את הנתיב ב-<bdi dir="ltr">\</bdi>.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor={id} className="sr-only">
          נתיב לבדיקה
        </label>
        <AutoGrowInput
          id={id}
          dir="ltr"
          joiner=""
          autoFocus
          className="min-w-48 flex-1 text-left font-mono"
          placeholder={sources[0] ? `${sources[0].replace(/\\+$/, "")}\\...` : "Documents\\~$report.docx"}
          value={probe.path}
          onChange={(path) => setProbe({ ...probe, path })}
        />
        <Button size="sm" icon={<File size={14} />} onClick={() => pick(false)}>
          קובץ...
        </Button>
        <Button size="sm" icon={<Folder size={14} />} onClick={() => pick(true)}>
          תיקייה...
        </Button>
      </div>
      {askValues && (any("largerThan") || any("olderThan") || any("hidden") || any("system")) && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <span className="text-muted">הקובץ לא נמצא בדיסק. פרטים לבדיקה:</span>
          {any("largerThan") && (
            <OptionalNumber label="גודל" unit="MB" value={probe.sizeMb} onChange={(sizeMb) => setProbe({ ...probe, sizeMb })} />
          )}
          {any("olderThan") && (
            <OptionalNumber
              label="שונה לפני"
              unit="ימים"
              value={probe.ageDays}
              onChange={(v) => setProbe({ ...probe, ageDays: v === null ? null : Math.round(v) })}
            />
          )}
          {any("hidden") && (
            <label className="flex cursor-pointer items-center gap-1.5">
              <Checkbox checked={probe.hidden} onChange={(hidden) => setProbe({ ...probe, hidden })} />
              מוסתר
            </label>
          )}
          {any("system") && (
            <label className="flex cursor-pointer items-center gap-1.5">
              <Checkbox checked={probe.system} onChange={(system) => setProbe({ ...probe, system })} />
              קובץ מערכת
            </label>
          )}
        </div>
      )}
      {check && check.kept !== null && (
        <>
          {check.asFolder && check.asFolder.kept !== null && (
            <div className="border-t border-line pt-2 text-muted">הנתיב לא נמצא בדיסק, אז זה נבדק גם כקובץ וגם כתיקייה.</div>
          )}
          <CheckResult
            c={check}
            title={check.asFolder ? "אם זה קובץ" : undefined}
            rules={rules}
            globalRules={globalRules}
            onAdd={onAdd}
            first
          />
          {check.asFolder && check.asFolder.kept !== null && (
            <CheckResult c={check.asFolder} title="אם זו תיקייה" rules={rules} globalRules={globalRules} onAdd={onAdd} />
          )}
        </>
      )}
      {check?.notes.map((n, i) => (
        <div key={i} className="flex items-start gap-1.5 text-muted">
          <Info size={13} className="mt-0.5 shrink-0" />
          <span>{n}</span>
        </div>
      ))}
    </div>
  );
}

const RELATION_TEXT: Record<Relation["kind"], string> = {
  same: "כפילות - כבר יש כלל שעושה אותו דבר:",
  inside: "כבר כלול בכלל:",
  contains: "כולל גם את הכלל (שהוא מיותר):",
  partial: "חלק מהערכים כבר כלולים בכלל:",
};

/** Under a rule: which other rules repeat it, include it, are included in it or overlap it. */
function RelationNotes({ related, onRemove }: { related: Relation[]; onRemove: () => void }) {
  if (related.length === 0) return null;
  const shown = related.slice(0, 3);
  return (
    <div className="flex flex-col gap-0.5 pb-1 pt-1.5 text-xs">
      {shown.map((x, i) => (
        <div key={i} className={`flex flex-wrap items-center gap-x-1.5 ${x.kind === "same" ? "text-warn" : "text-muted"}`}>
          {x.kind === "same" ? <Copy size={13} className="shrink-0" /> : <Info size={13} className="shrink-0" />}
          <span>
            {RELATION_TEXT[x.kind]} {describeFilter(x.other)}
            {x.global && " (כלל כללי)"}
          </span>
          {x.kind === "same" && !x.global && (
            <button type="button" onClick={onRemove} className="rounded px-1.5 py-0.5 font-medium text-accent underline hover:bg-hover">
              הסרת הכלל הזה
            </button>
          )}
        </div>
      ))}
      {related.length > shown.length && <div className="text-muted">ועוד {related.length - shown.length}...</div>}
    </div>
  );
}

function addSummary(a: Added) {
  const parts = [
    a.added.length > 0 && (a.added.length === 1 ? "נוסף כלל אחד" : `נוספו ${a.added.length} כללים`),
    a.same.length > 0 && `${a.same.length} כבר היו`,
    a.inside.length > 0 && `${a.inside.length} כבר כלולים בכללים קיימים`,
    a.partial.length > 0 && `${a.partial.length} חופפים חלקית לכלל קיים`,
  ].filter(Boolean);
  return parts.join(", ") || "לא נוספו כללים";
}

/** One rule on one line; its explanation sits behind the info button so a long list stays compact. */
function RuleRow({
  rule: r,
  onChange,
  onRemove,
  tested,
  related = [],
}: {
  rule: FilterRule;
  onChange: (r: FilterRule) => void;
  onRemove: () => void;
  /** What this rule does to the path in the filter check (when the check is open). */
  tested?: RuleResult;
  /** Rules that repeat, include or overlap this one. */
  related?: Relation[];
}) {
  // For a regex rule the explanation + live check open while its field is being edited, close when focus
  // leaves the row, and the info button brings them back. The try-out name outlives the panel.
  const [showHint, setShowHint] = useState(false);
  const [sample, setSample] = useState("");
  const isRegex = r.kind === "regex";
  const k = KINDS[r.kind];
  const hint = hintOf(r);
  // Switches without a field have nothing but the explanation to show, so it goes on the line itself.
  const inline = !hasText(r) && r.kind !== "largerThan" && r.kind !== "olderThan";
  const folders = r.kind === "regex" && r.folders;
  return (
    <li
      className="rounded-lg border border-line bg-panel2 px-3 py-1.5"
      onFocus={isRegex ? (e) => e.target instanceof HTMLInputElement && setShowHint(true) : undefined}
      onBlur={isRegex ? (e) => !e.currentTarget.contains(e.relatedTarget) && setShowHint(false) : undefined}
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
        <span className="flex w-36 shrink-0 items-center gap-2 text-[13px] font-medium">
          <span className="text-muted">{k.icon}</span>
          {k.label}
        </span>
        <div className="flex min-w-48 flex-1 flex-wrap items-center gap-2">
          {r.kind === "regex" && (
            <Segmented
              compact
              value={r.folders ? "folders" : "files"}
              onChange={(v) => onChange({ ...r, folders: v === "folders", include: v === "folders" ? false : r.include })}
              options={[
                { value: "files", label: "קבצים", icon: <FileType size={14} /> },
                { value: "folders", label: "תיקיות", icon: <FolderX size={14} /> },
              ]}
            />
          )}
          {(isFiles(r) || (r.kind === "regex" && !folders)) && (
            <Segmented
              compact
              value={isOnly(r) ? "only" : "skip"}
              onChange={(v) => {
                if (r.kind === "regex") onChange({ ...r, include: v === "only" });
                else if (isFiles(r)) onChange({ kind: filesKind(v === "only", isByName(r)), value: r.value });
              }}
              options={[
                { value: "skip", label: "לדלג על" },
                { value: "only", label: "לגבות רק" },
              ]}
            />
          )}
          {isFiles(r) && (
            <Segmented
              compact
              value={isByName(r) ? "name" : "ext"}
              onChange={(v) => {
                const byName = v === "name";
                if (byName === isByName(r)) return;
                onChange({ kind: filesKind(isOnly(r), byName), value: byName ? extsToNames(r.value) : namesToExts(r.value) });
              }}
              options={[
                { value: "ext", label: "סיומות", icon: <FileType size={14} /> },
                { value: "name", label: "שמות", icon: <Type size={14} /> },
              ]}
            />
          )}
          {hasText(r) &&
            (r.kind !== "regex" ? (
              // A list of extensions / names, or a path, wraps onto more lines instead of scrolling out of view.
              <AutoGrowInput
                dir="ltr"
                joiner={isFiles(r) ? ", " : ""}
                className="min-w-40 flex-1 text-left font-mono"
                autoFocus={!r.value}
                value={r.value}
                placeholder={PLACEHOLDERS[r.kind]}
                onChange={(value) => onChange({ ...r, value })}
              />
            ) : (
              <TextInput
                dir="ltr"
                className="min-w-40 flex-1 text-left font-mono"
                autoFocus={!r.value}
                value={r.value}
                placeholder={folders ? "^(build|cache)\\d*$" : PLACEHOLDERS[r.kind]}
                onChange={(e) => onChange({ ...r, value: e.target.value })}
              />
            ))}
          {r.kind === "largerThan" && (
            <div className="flex items-center gap-2">
              <NumberInput className="w-28" min={1} value={r.mb} onChange={(mb) => onChange({ ...r, mb })} />
              <span className="text-[13px] text-muted">MB</span>
            </div>
          )}
          {r.kind === "olderThan" && (
            <div className="flex items-center gap-2">
              <NumberInput className="w-28" min={1} value={r.days} onChange={(days) => onChange({ ...r, days })} />
              <span className="text-[13px] text-muted">ימים</span>
            </div>
          )}
          {inline && hint && <span className="text-xs text-muted">{hint}</span>}
        </div>
        {tested && <EffectBadge result={tested} />}
        {!inline && hint && (
          <IconButton
            label={showHint ? "הסתרת ההסבר" : "הסבר על הכלל"}
            onClick={() => setShowHint(!showHint)}
            aria-expanded={showHint}
          >
            <Info size={15} />
          </IconButton>
        )}
        <IconButton label="הסרת כלל" tone="danger" onClick={onRemove}>
          <Trash2 size={15} />
        </IconButton>
      </div>
      <RelationNotes related={related} onRemove={onRemove} />
      {r.kind === "regex" && showHint && <RegexCheckPanel rule={r} sample={sample} setSample={setSample} />}
      {/* A rule that was just added shows its explanation until it has a value. */}
      {!inline && hint && (showHint || (hasText(r) && !r.value)) && <p className="pb-1 pt-1.5 text-xs text-muted">{hint}</p>}
    </li>
  );
}

const NO_PROBE: FilterProbe = { path: "", sizeMb: null, ageDays: null, hidden: false, system: false };

export default function FilterRulesEditor({
  rules,
  onChange,
  emptyText = "אין כללי סינון - כל הקבצים מגובים.",
  sources = [],
  globalRules = [],
}: {
  rules: FilterRule[];
  onChange: (r: FilterRule[]) => void;
  emptyText?: string;
  /** The task's source folders: paths in the filter check are read under them. */
  sources?: string[];
  /** Rules that apply together with these (the global ones, in a task). */
  globalRules?: FilterRule[];
}) {
  const [testing, setTesting] = useState(false);
  const [probe, setProbe] = useState<FilterProbe>(NO_PROBE);
  const [check, setCheck] = useState<FilterCheck | null>(null);
  const key = JSON.stringify([rules, globalRules, sources, probe]);
  useEffect(() => {
    if (!testing || !probe.path.trim()) {
      setCheck(null);
      return;
    }
    let live = true;
    const t = setTimeout(() => {
      api
        .checkFilters(rules, globalRules, sources, probe)
        .then((c) => live && setCheck(c))
        .catch(() => live && setCheck(null));
    }, 200);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // `key` covers rules, globalRules, sources and probe (new arrays on every render).
  }, [testing, key]);
  // A rule removed or added shifts the results until the next check comes back.
  const tested = (i: number) =>
    check && check.kept !== null && check.rules.length === rules.length ? check.rules[i] : undefined;

  const update = (i: number, r: FilterRule) => onChange(rules.map((x, j) => (j === i ? r : x)));
  const remove = (i: number) => onChange(rules.filter((_, j) => j !== i));
  const singleton = (k: FilterKind) => (k === "hidden" || k === "system") && rules.some((r) => r.kind === k);

  const { toast, choose } = useFeedback();
  const related = useMemo(() => relations(rules, globalRules), [rules, globalRules]);
  const repeats = related.filter((r) => r.some((x) => x.kind === "same")).length;
  const covered = related.filter((r) => r.some((x) => x.kind === "inside")).length;

  /** Adds rules, leaving out those that already exist or that an existing rule includes, and says what happened. */
  const add = (incoming: FilterRule[], then?: (a: Added) => void) => {
    const a = addRules(rules, incoming);
    if (a.added.length) onChange(a.rules);
    if (incoming.length === 1 && !a.added.length) {
      const by = a.inside[0]?.by;
      const waiting = !by && hasText(a.same[0]) && !a.same[0].value;
      toast({
        tone: "info",
        title: by ? "הכלל כבר כלול בכלל אחר" : waiting ? "כבר יש כלל כזה שממתין להזנה" : "הכלל כבר קיים",
        message: by ? describeFilter(by) : waiting ? undefined : describeFilter(a.same[0]),
      });
    } else if (incoming.length > 1 || a.partial.length) {
      toast({ tone: a.added.length ? "ok" : "info", title: addSummary(a) });
    }
    then?.(a);
  };

  const tidyUp = () => {
    const t = tidy(rules);
    const left = relations(t.rules, globalRules).filter((r) => r.some((x) => x.kind === "inside")).length;
    const still = left > 0 ? ` ${left === 1 ? "כלל אחד עדיין כלול" : `${left} כללים עדיין כלולים`} בכלל אחר - הם מסומנים ברשימה.` : "";
    if (!t.changed) {
      toast({ tone: "info", title: "הכללים כבר מסודרים, בלי כפילויות.", message: still.trim() || undefined });
      return;
    }
    onChange(t.rules);
    const did = [
      t.merged > 0 && `מוזגו ${t.merged} כללים של סוגי קבצים ושמות`,
      t.repeated > 0 && `הוסרו ${t.repeated} כפילויות`,
    ].filter(Boolean);
    toast({ tone: "ok", title: "הכללים סודרו", message: `${did.length ? `${did.join(", ")}. ` : ""}${still}`.trim() || undefined });
  };

  const exportRules = async () => {
    try {
      const path = await api.saveFile("ייצוא כללי סינון", "Backuper - כללי סינון.json", "כללי סינון", ["json"]);
      if (!path) return;
      await api.exportFilters(path, rules);
      toast({ tone: "ok", title: `נשמרו ${rules.length} כללים לקובץ` });
    } catch (e) {
      toast({ tone: "bad", title: "הייצוא נכשל", message: errorText(e) });
    }
  };

  const importRules = async () => {
    try {
      const path = await api.pickFile("ייבוא כללי סינון", "כללי סינון", ["json"]);
      if (!path) return;
      const got = await api.importFilters(path);
      const left = got.skipped > 0 ? ` ${got.skipped} רשומות לא תקינות לא יובאו.` : "";
      if (!got.rules.length) {
        toast({ tone: "warn", title: "לא נמצאו כללים תקינים בקובץ", message: left.trim() || undefined });
        return;
      }
      const mode =
        rules.length === 0
          ? "add"
          : await choose({
              title: "ייבוא כללי סינון",
              message: `בקובץ ${got.rules.length} כללים, ובעורך כבר ${rules.length}. כללים שכבר קיימים או שכלולים בכלל קיים לא יתווספו.`,
              options: [
                { value: "add", label: "הוספה לכללים הקיימים" },
                { value: "replace", label: "החלפת הכללים הקיימים", danger: true },
              ],
            });
      if (!mode) return;
      const a = addRules(mode === "replace" ? [] : rules, got.rules);
      onChange(a.rules);
      toast({ tone: a.added.length ? "ok" : "info", title: addSummary(a), message: left.trim() || undefined });
    } catch (e) {
      toast({ tone: "bad", title: "הייבוא נכשל", message: errorText(e) });
    }
  };

  return (
    <div className="flex flex-col gap-2">
      {(repeats > 0 || covered > 0) && (
        <div className="flex flex-wrap items-center gap-2 rounded-lg bg-warn-soft px-3 py-1.5 text-xs">
          <AlertTriangle size={14} className="shrink-0 text-warn" />
          <span className="flex-1">
            {[
              repeats > 0 && (repeats === 1 ? "כלל כפול אחד" : `${repeats} כללים כפולים`),
              covered > 0 && (covered === 1 ? "כלל אחד שכלול בכלל אחר" : `${covered} כללים שכלולים בכלל אחר`),
            ]
              .filter(Boolean)
              .join(", ")}
            . ההסברים מופיעים מתחת לכל כלל.
          </span>
          {repeats > 0 && (
            <Button size="sm" onClick={tidyUp}>
              הסרת כפילויות
            </Button>
          )}
        </div>
      )}
      {rules.length === 0 ? (
        <div className="flex items-center gap-2 rounded-lg border border-dashed border-line px-3 py-3 text-[13px] text-muted">
          <Filter size={15} />
          {emptyText}
        </div>
      ) : (
        <ul className="flex flex-col gap-2">
          {rules.map((r, i) => (
            <RuleRow
              key={i}
              rule={r}
              onChange={(n) => update(i, n)}
              onRemove={() => remove(i)}
              tested={tested(i)}
              related={related[i]}
            />
          ))}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Menu
          trigger={(open) => (
            <Button size="sm" icon={<Plus size={15} />} onClick={open}>
              הוספת כלל
            </Button>
          )}
          items={MENU.map((kind) => ({
            label: KINDS[kind].label,
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
        <div className="ms-auto flex flex-wrap items-center gap-2">
          {rules.length > 1 && (
            <Button size="sm" variant="ghost" icon={<ArrowDownUp size={15} />} onClick={tidyUp} title="מיון לפי סוג, מיזוג כללים של סוגי קבצים ושמות והסרת כפילויות">
              סידור הכללים
            </Button>
          )}
          <Menu
            trigger={(open) => (
              <Button size="sm" variant="ghost" icon={<Download size={15} />} onClick={open}>
                ייבוא וייצוא
              </Button>
            )}
            items={[
              { label: "ייצוא הכללים לקובץ...", icon: <Download size={15} />, disabled: rules.length === 0, onClick: exportRules },
              { label: "ייבוא כללים מקובץ...", icon: <Upload size={15} />, onClick: importRules },
            ]}
          />
          {!testing && (
            <Button size="sm" variant="ghost" icon={<TestTube size={15} />} onClick={() => setTesting(true)}>
              בדיקת הכללים על קובץ
            </Button>
          )}
        </div>
      </div>
      {testing && (
        <FilterCheckPanel
          rules={rules}
          globalRules={globalRules}
          sources={sources}
          probe={probe}
          setProbe={setProbe}
          check={check}
          onAdd={add}
          onClose={() => setTesting(false)}
        />
      )}
    </div>
  );
}
