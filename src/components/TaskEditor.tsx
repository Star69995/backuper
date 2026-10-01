import { AlertCircle, ChevronDown, Copy, FolderOpen, Layers, RefreshCw } from "lucide-react";
import { type ReactNode, useState } from "react";
import { api, errorText } from "../api";
import { exampleFolderName, joinPath, sanitizeFolderName } from "../lib/format";
import type { BackupMode, Task } from "../types";
import FilterRulesEditor from "./FilterRulesEditor";
import ScheduleEditor from "./ScheduleEditor";
import { Button, cx, Field, Modal, NumberInput, TextInput, Toggle } from "./ui";

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
      <TextInput dir="ltr" className="text-left" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
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

export function ModeCard({
  mode,
  selected,
  onSelect,
}: {
  mode: BackupMode;
  selected: boolean;
  onSelect: () => void;
}) {
  const info =
    mode === "full"
      ? {
          icon: <Copy size={18} />,
          title: "מלא",
          text: "בכל ריצה נוצרת תיקייה חדשה עם התאריך, זהה לגמרי למקור (קבצים שנמחקו במקור לא יופיעו). הגיבוי הקודם נמחק אחרי הצלחה.",
        }
      : {
          icon: <Layers size={18} />,
          title: "אינקרמנטלי",
          text: "מעתיק רק קבצים חדשים או ששונו אל תיקיית הגיבוי האחרונה. מהיר מאוד. קבצים שנמחקו במקור נשארים בגיבוי.",
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
  const [folderTouched, setFolderTouched] = useState(!isNew);
  const [advanced, setAdvanced] = useState(task.copyEmptyDirs);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = <K extends keyof Task>(k: K, v: Task[K]) => setT((x) => ({ ...x, [k]: v }));
  const dirty = JSON.stringify(t) !== JSON.stringify(task);
  const locationChanged = !isNew && (t.destination !== task.destination || sanitizeFolderName(t.folderName) !== task.folderName);

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
              <TextInput
                autoFocus
                value={t.name}
                placeholder="למשל: מסמכים"
                onChange={(e) => {
                  const name = e.target.value;
                  setT((x) => ({ ...x, name, folderName: folderTouched ? x.folderName : sanitizeFolderName(name) }));
                }}
              />
            </Field>
            <div className="pb-2">
              <Toggle checked={t.enabled} onChange={(v) => set("enabled", v)} label="משימה פעילה" />
            </div>
          </div>
        </Section>

        <Section title="מקור ויעד">
          <Field label="תיקיית מקור" hint="התיקייה שאותה מגבים">
            <PathPicker value={t.source} onChange={(v) => set("source", v)} title="בחירת תיקיית מקור" placeholder="C:\Users\...\Documents" />
          </Field>
          <Field label="תיקיית יעד" hint="בתוכה תיווצר תיקיית גיבוי עם תאריך">
            <PathPicker value={t.destination} onChange={(v) => set("destination", v)} title="בחירת תיקיית יעד" placeholder="D:\Backups" />
          </Field>
          <Field
            label="שם תיקיות הגיבוי"
            hint={
              <span>
                לדוגמה:{" "}
                <bdi dir="ltr" className="selectable">
                  {joinPath(t.destination || "D:\\Backups", "")}
                  <bdi>{exampleFolderName(t.folderName || t.name)}</bdi>
                </bdi>
              </span>
            }
          >
            <TextInput
              value={t.folderName}
              placeholder={t.name || "שם"}
              onChange={(e) => {
                setFolderTouched(true);
                set("folderName", e.target.value);
              }}
            />
          </Field>
          {locationChanged && (
            <div className="flex items-start gap-2 rounded-lg bg-warn-soft px-3 py-2 text-[13px] text-warn">
              <AlertCircle size={16} className="mt-0.5 shrink-0" />
              שיניתם את היעד או את שם התיקיות. גיבויים שכבר קיימים במיקום הקודם לא ינוהלו יותר על ידי המשימה (ולא יימחקו
              אוטומטית).
            </div>
          )}
        </Section>

        <Section title="סוג גיבוי">
          <div className="flex flex-col gap-3 sm:flex-row" role="radiogroup">
            <ModeCard mode="incremental" selected={t.mode === "incremental"} onSelect={() => set("mode", "incremental")} />
            <ModeCard mode="full" selected={t.mode === "full"} onSelect={() => set("mode", "full")} />
          </div>
          {t.mode === "incremental" ? (
            <Field
              label="גיבוי מלא חדש כל"
              hint="יוצר מדי פעם תיקיית גיבוי חדשה ונקייה (ומוחק את הקודמת). 0 = אף פעם, הגיבוי המלא נעשה רק בפעם הראשונה."
            >
              <div className="flex items-center gap-2">
                <NumberInput className="w-24" min={0} value={t.fullEveryDays} onChange={(v) => set("fullEveryDays", v)} />
                <span className="text-sm text-muted">ימים</span>
              </div>
            </Field>
          ) : (
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
                  : "במקום להעתיק הכול מחדש, הגיבוי הקודם מקבל את התאריך החדש ומתעדכן להיות זהה למקור. חוסך זמן ומקום, אבל בזמן הריצה אין עותק שלם נוסף."
              }
            />
          )}
          <Field label="מספר גיבויים מלאים לשמירה" hint="1 = כל גיבוי מלא חדש מוחק את הקודם (אחרי שהצליח).">
            <NumberInput className="w-24" min={1} max={100} value={t.keepCount} onChange={(v) => set("keepCount", v)} />
          </Field>
        </Section>

        <Section title="תזמון">
          <ScheduleEditor value={t.schedule} onChange={(s) => set("schedule", s)} />
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
