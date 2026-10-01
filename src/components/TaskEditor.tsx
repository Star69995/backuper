import {
  AlertCircle,
  AlertTriangle,
  CalendarRange,
  ChevronDown,
  Copy,
  FolderOpen,
  FolderPlus,
  Layers,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { api, errorText } from "../api";
import {
  defaultFolderName,
  exampleFolderName,
  joinPath,
  kindFields,
  sanitizeFolderName,
  type TaskKind,
  taskKind,
} from "../lib/format";
import type { BackupMode, Source, Task } from "../types";
import FilterRulesEditor from "./FilterRulesEditor";
import ScheduleEditor from "./ScheduleEditor";
import { Button, cx, Field, IconButton, Modal, NumberInput, TextInput, Toggle } from "./ui";

export function Section({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  return (
    <section className={cx("flex flex-col gap-3", className)}>
      <h3 className="text-[13px] font-semibold text-muted">{title}</h3>
      {children}
    </section>
  );
}

export function PathPicker({
  value,
  onChange,
  title,
  placeholder,
}: {
  value: string;
  onChange: (v: string) => void;
  title: string;
  placeholder: string;
}) {
  return (
    <div className="flex gap-2">
      <TextInput
        dir="ltr"
        className="text-left"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      <Button
        icon={<FolderOpen size={16} />}
        onClick={async () => {
          const p = await api.pickFolder(title, value);
          if (p) onChange(p);
        }}
      >
        עיון
      </Button>
    </div>
  );
}

export function ModeCard({ kind, selected, onSelect }: { kind: TaskKind; selected: boolean; onSelect: () => void }) {
  const info =
    kind === "combined"
      ? {
          icon: <CalendarRange size={18} />,
          title: "משולב",
          text: "גיבוי מלא לפי תזמון משלו (למשל פעם בשבוע), ובשאר הזמן גיבויים אינקרמנטליים בלבד. כל גיבוי מלא פותח רצף חדש.",
        }
      : kind === "full"
        ? {
            icon: <Copy size={18} />,
            title: "מלא",
            text: "בכל ריצה נוצרת תיקייה חדשה עם התאריך, זהה לגמרי למקור (קבצים שנמחקו במקור לא יופיעו). הגיבויים הקודמים נמחקים.",
          }
        : {
            icon: <Layers size={18} />,
            title: "אינקרמנטלי",
            text: "בכל ריצה נוצרת תיקייה חדשה עם התאריך, ובה רק קבצים חדשים או ששונו מאז הגיבוי הקודם. אם לא היו שינויים, התיקייה ריקה ותימחק בגיבוי המלא הבא. הגיבוי הראשון תמיד מלא.",
          };
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={cx(
        "flex flex-1 flex-col gap-1.5 rounded-xl border p-3 text-start transition-colors",
        selected ? "border-accent bg-accent-soft/60 ring-1 ring-accent" : "border-line hover:bg-hover",
      )}
    >
      <span className={cx("flex items-center gap-2 font-semibold", selected && "text-accent")}>
        {info.icon}
        {info.title}
      </span>
      <span className="text-xs leading-relaxed text-muted">{info.text}</span>
    </button>
  );
}

/** Adds " (2)", " (3)"... until the folder name is unique among the task's sources. */
function uniqueName(name: string, taken: string[]) {
  const lower = taken.map((t) => t.toLowerCase());
  if (!lower.includes(name.toLowerCase())) return name;
  let i = 2;
  while (lower.includes(`${name} (${i})`.toLowerCase())) i++;
  return `${name} (${i})`;
}

function SourcesEditor({
  sources,
  destination,
  mode,
  onChange,
}: {
  sources: Source[];
  destination: string;
  mode: BackupMode;
  onChange: (s: Source[]) => void;
}) {
  const update = (i: number, s: Source) => onChange(sources.map((x, j) => (j === i ? s : x)));
  const others = (i: number) => sources.filter((_, j) => j !== i).map((s) => s.folderName);
  const setPath = (i: number, path: string) => {
    const cur = sources[i];
    // Keep the folder name in sync with the path until the user names it themselves.
    const auto = !cur.folderName || cur.folderName === defaultFolderName(cur.path);
    update(i, { path, folderName: auto ? uniqueName(defaultFolderName(path), others(i)) : cur.folderName });
  };
  const add = async () => {
    const paths = await api.pickFolders("הוספת תיקיות מקור (אפשר לבחור כמה עם Ctrl)");
    const next = [...sources];
    for (const path of paths) {
      if (next.some((s) => s.path.toLowerCase() === path.toLowerCase())) continue;
      next.push({
        path,
        folderName: uniqueName(
          defaultFolderName(path),
          next.map((s) => s.folderName),
        ),
      });
    }
    if (next.length !== sources.length) onChange(next);
  };
  const example = sources.find((s) => s.folderName)?.folderName ?? "Documents";

  return (
    <div className="flex flex-col gap-2">
      {sources.length > 0 && (
        <div className="hidden gap-2 px-1 text-xs font-medium text-muted sm:flex">
          <span className="flex-1">תיקיית מקור</span>
          <span className="w-44">שם תיקיות הגיבוי</span>
          <span className="w-8" />
        </div>
      )}
      {sources.length === 0 && (
        <div className="rounded-lg border border-dashed border-line px-3 py-3 text-[13px] text-muted">
          עדיין לא נבחרו תיקיות. אפשר להוסיף כמה תיקיות מקור - כולן יגובו יחד לאותו יעד.
        </div>
      )}
      {sources.map((src, i) => (
        <div
          key={i}
          className="flex flex-col gap-2 rounded-lg border border-line bg-panel2 p-2 sm:flex-row sm:items-center sm:border-0 sm:bg-transparent sm:p-0"
        >
          <div className="min-w-0 flex-1">
            <PathPicker
              value={src.path}
              onChange={(v) => setPath(i, v)}
              title="בחירת תיקיית מקור"
              placeholder="C:\Users\...\Documents"
            />
          </div>
          <div className="flex items-center gap-2">
            <TextInput
              aria-label="שם תיקיות הגיבוי"
              className="w-44"
              value={src.folderName}
              placeholder={defaultFolderName(src.path) || "שם"}
              onChange={(e) => update(i, { ...src, folderName: e.target.value })}
            />
            <IconButton label="הסרת תיקיית המקור" tone="danger" onClick={() => onChange(sources.filter((_, j) => j !== i))}>
              <Trash2 size={16} />
            </IconButton>
          </div>
        </div>
      ))}
      <div className="flex flex-wrap items-center gap-3">
        <Button size="sm" icon={<FolderPlus size={15} />} onClick={add}>
          הוספת תיקיית מקור
        </Button>
        {sources.length > 1 && <span className="text-xs text-muted">כל התיקיות מגובות יחד, באותו תזמון ועם אותו תאריך.</span>}
      </div>
      <p className="text-xs text-muted">
        לכל תיקיית מקור יש גיבויים משלה ביעד, לדוגמה:{" "}
        <bdi dir="ltr" className="selectable">
          {joinPath(destination || "D:\\Backups", "")}
          <bdi>{exampleFolderName(example, mode)}</bdi>
        </bdi>
      </p>
    </div>
  );
}

export default function TaskEditor({
  task,
  globalFilterCount,
  onClose,
  onSaved,
}: {
  task: Task;
  globalFilterCount: number;
  onClose: () => void;
  onSaved: (t: Task, isNew: boolean) => void;
}) {
  const isNew = !task.id;
  const [t, setT] = useState<Task>(task);
  const [advanced, setAdvanced] = useState(task.copyEmptyDirs);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = <K extends keyof Task>(k: K, v: Task[K]) => setT((x) => ({ ...x, [k]: v }));
  const kind = taskKind(t);
  const dirty = JSON.stringify(t) !== JSON.stringify(task);
  // Existing backups are found by destination + folder name; changing either orphans them.
  const locationChanged =
    !isNew &&
    (t.destination !== task.destination ||
      task.sources.some((old) => {
        const cur = t.sources.find((s) => s.path === old.path);
        return cur && sanitizeFolderName(cur.folderName) !== old.folderName;
      }));

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const [saved] = await api.saveTasks([t]);
      onSaved(saved, isNew);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      size="lg"
      dismissable={!dirty}
      title={isNew ? "משימת גיבוי חדשה" : `עריכת משימה: ${task.name}`}
      onClose={onClose}
      footer={
        <>
          {error && (
            <div className="me-auto flex items-center gap-1.5 text-[13px] text-bad">
              <AlertCircle size={16} className="shrink-0" />
              {error}
            </div>
          )}
          <Button onClick={onClose}>ביטול</Button>
          <Button variant="primary" onClick={save} disabled={saving}>
            {isNew ? "צור משימה" : "שמור שינויים"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-6">
        <Section title="כללי">
          <div className="flex flex-wrap items-end gap-4">
            <Field label="שם המשימה" className="min-w-60 flex-1">
              <TextInput autoFocus value={t.name} placeholder="למשל: מסמכים" onChange={(e) => set("name", e.target.value)} />
            </Field>
            <div className="pb-2">
              <Toggle checked={t.enabled} onChange={(v) => set("enabled", v)} label="משימה פעילה" />
            </div>
          </div>
        </Section>

        <Section title="תיקיות מקור">
          <SourcesEditor sources={t.sources} destination={t.destination} mode={t.mode} onChange={(v) => set("sources", v)} />
        </Section>

        <Section title="יעד">
          <Field label="תיקיית יעד" hint="בתוכה נוצרות תיקיות הגיבוי עם התאריך, בנפרד לכל תיקיית מקור">
            <PathPicker
              value={t.destination}
              onChange={(v) => set("destination", v)}
              title="בחירת תיקיית יעד"
              placeholder="D:\Backups"
            />
          </Field>
          {locationChanged && (
            <div className="flex items-start gap-2 rounded-lg bg-warn-soft px-3 py-2 text-[13px] text-warn">
              <AlertCircle size={16} className="mt-0.5 shrink-0" />
              שיניתם את היעד או שם של תיקיות גיבוי. גיבויים שכבר קיימים במיקום הקודם לא ינוהלו יותר על ידי המשימה (ולא יימחקו
              אוטומטית).
            </div>
          )}
        </Section>

        <Section title="סוג גיבוי">
          <div className="flex flex-col gap-3 sm:flex-row" role="radiogroup">
            {(["incremental", "combined", "full"] as const).map((k) => (
              <ModeCard
                key={k}
                kind={k}
                selected={kind === k}
                onSelect={() => setT((x) => ({ ...x, ...kindFields(k, x.fullSchedule ?? task.fullSchedule) }))}
              />
            ))}
          </div>
          {kind !== "full" && (
            <Toggle
              checked={t.deleteEmptyIncrementals}
              onChange={(v) => set("deleteEmptyIncrementals", v)}
              label="מחיקת תיקיות אינקרמנטליות ריקות בגיבוי מלא"
              description="תיקייה אינקרמנטלית ריקה נוצרת כשלא היו שינויים. כשכבויה - התיקיות הריקות נשארות כתיעוד לריצות, עד שהרצף שלהן נמחק (לפי מספר הגיבויים לשמירה)."
            />
          )}
          {kind !== "incremental" && (
            <Toggle
              checked={t.reusePrevious}
              onChange={(v) => set("reusePrevious", v)}
              disabled={t.keepCount > 1}
              label={
                <span className="flex items-center gap-1.5">
                  <RefreshCw size={14} /> גיבוי מלא מהיר
                </span>
              }
              description={
                t.keepCount > 1
                  ? "זמין רק כששומרים גיבוי אחד."
                  : "במקום להעתיק הכול מחדש, הגיבוי המלא הקודם מקבל את התאריך החדש ומתעדכן להיות זהה למקור. חוסך זמן ומקום, אבל בזמן הריצה אין עותק שלם נוסף."
              }
            />
          )}
          <Field
            label="מספר גיבויים מלאים לשמירה"
            hint="1 = כל גיבוי מלא חדש מוחק את הקודם, יחד עם הגיבויים האינקרמנטליים שאחריו."
          >
            <NumberInput className="w-24" min={1} max={100} value={t.keepCount} onChange={(v) => set("keepCount", v)} />
          </Field>
          <Toggle
            checked={t.deleteBefore}
            onChange={(v) => set("deleteBefore", v)}
            label="מחיקת הגיבויים הקודמים לפני תחילת גיבוי מלא"
            description="מפנה מקום בדיסק לפני ההעתקה, כשאין מספיק מקום לשני עותקים. כברירת מחדל הגיבויים הקודמים נמחקים רק אחרי שהגיבוי החדש הצליח."
          />
          {t.deleteBefore && (
            <div className="flex items-start gap-2 rounded-lg bg-warn-soft px-3 py-2 text-[13px] text-warn">
              <AlertTriangle size={16} className="mt-0.5 shrink-0" />
              שימו לב: אם הגיבוי המלא ייכשל או יבוטל באמצע, לא יישאר גיבוי שלם קודם.
            </div>
          )}
        </Section>

        <Section title="תזמון">
          {kind === "combined" ? (
            <>
              <div className="flex flex-col gap-2 rounded-xl border border-line p-3">
                <h4 className="text-sm font-semibold">גיבויים אינקרמנטליים</h4>
                <ScheduleEditor value={t.schedule} onChange={(s) => set("schedule", s)} />
              </div>
              <div className="flex flex-col gap-2 rounded-xl border border-accent/30 p-3">
                <h4 className="text-sm font-semibold text-accent">גיבוי מלא</h4>
                <ScheduleEditor value={t.fullSchedule!} onChange={(s) => set("fullSchedule", s)} allowManual={false} />
              </div>
              <p className="text-xs text-muted">אם שני התזמונים חלים באותו זמן, רץ רק הגיבוי המלא.</p>
            </>
          ) : (
            <ScheduleEditor value={t.schedule} onChange={(s) => set("schedule", s)} />
          )}
          <Toggle
            checked={t.catchUp}
            onChange={(v) => set("catchUp", v)}
            label="השלמת גיבוי שהוחמץ"
            description="אם המחשב היה כבוי או במצב שינה בזמן המתוזמן, הגיבוי ירוץ מיד כשהתוכנה עולה."
          />
        </Section>

        <Section title="סינון קבצים - מה לא לגבות">
          <FilterRulesEditor rules={t.filters} onChange={(f) => set("filters", f)} />
          <Toggle
            checked={t.useGlobalFilters}
            onChange={(v) => set("useGlobalFilters", v)}
            label="החלת כללי הסינון הכלליים"
            description={
              globalFilterCount === 0
                ? "עדיין לא הוגדרו כללים כלליים (אפשר להגדיר אותם בהגדרות)."
                : `${globalFilterCount} כללים שמוגדרים בהגדרות וחלים על כל המשימות.`
            }
          />
        </Section>

        <section className="flex flex-col gap-3">
          <button
            type="button"
            onClick={() => setAdvanced((a) => !a)}
            className="flex items-center gap-1.5 self-start text-[13px] font-semibold text-muted hover:text-fg"
          >
            <ChevronDown size={16} className={cx("transition-transform", advanced && "rotate-180")} />
            אפשרויות מתקדמות
          </button>
          {advanced && (
            <div className="flex flex-col gap-4 rounded-xl border border-line p-4">
              <Toggle
                checked={t.copyEmptyDirs}
                onChange={(v) => set("copyEmptyDirs", v)}
                label="העתקת תיקיות ריקות"
                description="כברירת מחדל תיקיות ריקות לא מועתקות לגיבוי."
              />
            </div>
          )}
        </section>
      </div>
    </Modal>
  );
}
