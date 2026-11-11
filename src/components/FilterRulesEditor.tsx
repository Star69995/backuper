import {
  AlertTriangle,
  CalendarX,
  Check,
  CircleHelp,
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
  X,
} from "lucide-react";
import { useEffect, useId, useState, type ReactNode } from "react";
import { api } from "../api";
import type { FilterCheck, FilterKind, FilterProbe, FilterRule, RegexCheck, RuleResult } from "../types";
import { Badge, Button, Checkbox, IconButton, Menu, NumberInput, Segmented, TextInput } from "./ui";

const KINDS: Record<FilterKind, { label: string; icon: ReactNode; hint?: string }> = {
  // The three file-name kinds share one row (see isFiles); "include" is its "only these" mode.
  include: { label: "סוגי קבצים ושמות", icon: <FileType size={15} /> },
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
    hint: "התיקייה ומה שבתוכה לא יגובו. שם תיקייה בכל מקום בעץ (node_modules) או נתיב מלא.",
  },
  file: {
    label: "קובץ מסוים",
    icon: <FileX size={15} />,
    hint: "הקובץ הזה לא יגובה. נתיב מלא, למשל C:\\pagefile.sys. נוח לקובץ שנכשל בכל גיבוי כי הוא תמיד נעול.",
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

const isFiles = (r: FilterRule): r is Extract<FilterRule, { kind: "include" | "extension" | "pattern" }> =>
  r.kind === "include" || r.kind === "extension" || r.kind === "pattern";

/** "Only these" (back up only matches) vs "skip these". */
const isOnly = (r: FilterRule) => r.kind === "include" || (r.kind === "regex" && r.include);

const FILES_HINT = {
  skip: "קבצים עם הסיומות או השמות האלה לא יגובו. מפרידים בפסיק, למשל tmp, log, Thumbs.db. הסימן * מייצג כל טקסט: ~$*",
  only: "רק קבצים עם הסיומות או השמות האלה יגובו, כל השאר לא. מפרידים בפסיק, למשל docx, xlsx. הסימן * מייצג כל טקסט.",
};

const REGEX_FOLDERS_HINT =
  "הביטוי נבדק מול שם התיקייה בלבד, בכל עומק בעץ, בלי הבחנה בין אותיות גדולות לקטנות. תיקייה תואמת ומה שבתוכה לא יגובו. למשל ⁦^(build|cache)\\d*$⁩ - build, cache2 ועוד. אפשר רק לדלג על תיקיות, לא לגבות רק אותן.";

const PLACEHOLDERS: Partial<Record<FilterKind, string>> = {
  include: "docx, xlsx, jpg",
  extension: "tmp, log, ~$*",
  pattern: "~$*",
  regex: "^IMG_\\d+",
  folder: "node_modules",
  file: "C:\\pagefile.sys",
};

/** The explanation of a rule, shown on demand (and right away while the rule is still empty). */
const hintOf = (r: FilterRule) => {
  if (isFiles(r)) return FILES_HINT[isOnly(r) ? "only" : "skip"];
  if (r.kind === "regex" && r.folders) return REGEX_FOLDERS_HINT;
  return KINDS[r.kind].hint;
};

const hasText = (r: FilterRule): r is Extract<FilterRule, { value: string }> => "value" in r;

const blank = (kind: FilterKind): FilterRule => {
  switch (kind) {
    case "include":
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

const same = (a: FilterRule, b: FilterRule) => JSON.stringify(a) === JSON.stringify(b);

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
  onClose,
}: {
  rules: FilterRule[];
  globalRules: FilterRule[];
  sources: string[];
  probe: FilterProbe;
  setProbe: (p: FilterProbe) => void;
  check: FilterCheck | null;
  onClose: () => void;
}) {
  const id = useId();
  const any = (k: FilterKind) => rules.some((r) => r.kind === k) || globalRules.some((r) => r.kind === k);
  // Size, date and attributes are asked only for a file that isn't on disk, and only when a rule looks at them.
  const askValues = check?.kept != null && !check.onDisk && !check.folder;
  const subject = check?.folder ? "התיקייה" : "הקובץ";
  const pick = async (folder: boolean) => {
    const p = folder
      ? await api.pickFolder("בחירת תיקייה לבדיקה", sources[0])
      : await api.pickFile("בחירת קובץ לבדיקה", "כל הקבצים", ["*"], sources[0]);
    if (p) setProbe({ ...probe, path: folder ? `${p.replace(/\\+$/, "")}\\` : p });
  };
  const affecting = [
    ...rules.map((r, i) => ({ rule: r, result: check?.rules[i], global: false })),
    ...globalRules.map((r, i) => ({ rule: r, result: check?.globalRules[i], global: true })),
  ].filter((x) => x.result && x.result.effect !== "none");
  const quiet = rules.length + globalRules.length - affecting.length;

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
        <TextInput
          id={id}
          dir="ltr"
          autoFocus
          className="min-w-48 flex-1 text-left font-mono"
          placeholder={sources[0] ? `${sources[0].replace(/\\+$/, "")}\\...` : "Documents\\~$report.docx"}
          value={probe.path}
          onChange={(e) => setProbe({ ...probe, path: e.target.value })}
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
        <div className="flex flex-col gap-1.5 border-t border-line pt-2">
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone={check.kept ? "ok" : "warn"} icon={check.kept ? <Check size={12} /> : <X size={12} />}>
              {check.kept ? `${subject} יגובה` : `${subject} לא יגובה`}
            </Badge>
            {check.reason && <span>{check.reason}</span>}
          </div>
          {check.onDisk && (
            <div className="text-muted">
              נבדק מול {subject} שבדיסק
              {!check.folder && (
                <>
                  : <bdi dir="ltr">{fmtSize(check.onDisk.size)}</bdi>
                  {check.onDisk.modified && `, שונה ב-${new Date(check.onDisk.modified).toLocaleDateString("he-IL")}`}
                  {check.onDisk.hidden && ", מוסתר"}
                  {check.onDisk.system && ", קובץ מערכת"}
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
          {rules.length + globalRules.length === 0 && <div className="text-muted">אין כללי סינון, אז כל הקבצים מגובים.</div>}
        </div>
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

/** One rule on one line; its explanation sits behind the info button so a long list stays compact. */
function RuleRow({
  rule: r,
  onChange,
  onRemove,
  tested,
}: {
  rule: FilterRule;
  onChange: (r: FilterRule) => void;
  onRemove: () => void;
  /** What this rule does to the path in the filter check (when the check is open). */
  tested?: RuleResult;
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
                else onChange({ kind: v === "only" ? "include" : r.kind === "include" ? "extension" : r.kind, value: r.value });
              }}
              options={[
                { value: "skip", label: "לדלג על" },
                { value: "only", label: "לגבות רק" },
              ]}
            />
          )}
          {hasText(r) && (
            <TextInput
              dir="ltr"
              className="min-w-40 flex-1 text-left font-mono"
              autoFocus={!r.value}
              value={r.value}
              placeholder={folders ? "^(build|cache)\\d*$" : PLACEHOLDERS[r.kind]}
              onChange={(e) => onChange({ ...r, value: e.target.value })}
            />
          )}
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
          {rules.map((r, i) => (
            <RuleRow key={i} rule={r} onChange={(n) => update(i, n)} onRemove={() => remove(i)} tested={tested(i)} />
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
        {!testing && (
          <Button size="sm" variant="ghost" icon={<TestTube size={15} />} onClick={() => setTesting(true)} className="ms-auto">
            בדיקת הכללים על קובץ
          </Button>
        )}
      </div>
      {testing && (
        <FilterCheckPanel
          rules={rules}
          globalRules={globalRules}
          sources={sources}
          probe={probe}
          setProbe={setProbe}
          check={check}
          onClose={() => setTesting(false)}
        />
      )}
    </div>
  );
}
