import { getVersion } from "@tauri-apps/api/app";
import { Bell, Download, Filter, FolderSearch, History, Import, Monitor, Moon, Power, RotateCcw, Sun, Volume2 } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtRelative, fmtSmart } from "../lib/format";
import type { Settings, Snapshot, TaskSnapshot } from "../types";
import { useFeedback } from "./feedback";
import FilterRulesEditor from "./FilterRulesEditor";
import { useTaskListTransfer } from "./ImportTasksDialog";
import { PathPicker } from "./TaskEditor";
import { Button, IconButton, Modal, PathText, Segmented, Select, Toggle } from "./ui";

function Card({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-xl border border-line bg-panel p-5">
      <h2 className="mb-4 flex items-center gap-2 font-semibold">
        <span className="text-muted">{icon}</span>
        {title}
      </h2>
      <div className="flex flex-col gap-5">{children}</div>
    </section>
  );
}

export default function SettingsView({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast } = useFeedback();
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion);
  }, []);

  const s = snap.settings;
  const update = (patch: Partial<Settings>) =>
    api
      .saveSettings({ ...s, ...patch })
      .then(refresh)
      .catch((e) => {
        toast({ tone: "bad", title: "השמירה נכשלה", message: errorText(e) });
        throw e;
      });

  // Filter rules are edited as a draft and saved explicitly (a new rule starts empty).
  const [filters, setFilters] = useState(s.globalFilters);
  const filtersDirty = JSON.stringify(filters) !== JSON.stringify(s.globalFilters);

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-4 p-4 md:p-6">
      <header>
        <h1 className="text-xl font-semibold">הגדרות</h1>
      </header>

      <Card title="מראה" icon={<Sun size={18} />}>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <span className="text-sm font-medium">ערכת צבעים</span>
          <Segmented
            value={s.theme}
            onChange={(theme) => update({ theme })}
            options={[
              { value: "system", label: "לפי Windows", icon: <Monitor size={14} /> },
              { value: "light", label: "בהיר", icon: <Sun size={14} /> },
              { value: "dark", label: "כהה", icon: <Moon size={14} /> },
            ]}
          />
        </div>
      </Card>

      <Card title="הפעלה ורקע" icon={<Power size={18} />}>
        <Toggle
          checked={snap.autostart}
          onChange={(v) =>
            api
              .setAutostart(v)
              .then(refresh)
              .catch((e) => toast({ tone: "bad", title: "השינוי נכשל", message: errorText(e) }))
          }
          label="הפעלה עם הדלקת המחשב"
          description="התוכנה עולה אוטומטית לאזור ההודעות (ליד השעון) ומתחילה לבצע את התזמונים."
        />
        <Toggle
          checked={s.closeToTray}
          onChange={(closeToTray) => update({ closeToTray })}
          label="סגירת החלון ממזערת לאזור ההודעות"
          description="התזמון ממשיך לפעול ברקע. ליציאה מלאה: לחיצה ימנית על הסמל ליד השעון - יציאה. אם האפשרות כבויה, סגירת החלון עוצרת את התזמון."
        />
        <Toggle
          checked={s.schedulerPaused}
          onChange={(schedulerPaused) => update({ schedulerPaused })}
          label="השהיית כל התזמונים"
          description="משימות לא ירוצו אוטומטית עד לחידוש. אפשר עדיין להריץ ידנית."
        />
      </Card>

      <Card title="כללי סינון כלליים" icon={<Filter size={18} />}>
        <p className="-mt-2 text-[13px] text-muted">
          קבצים שתואמים לכללים האלה לא יגובו באף משימה (אלא אם כיבו במשימה את "החלת כללי הסינון הכלליים").
        </p>
        <FilterRulesEditor rules={filters} onChange={setFilters} emptyText="אין כללים כלליים." />
        {filtersDirty && (
          <div className="flex justify-end gap-2">
            <Button size="sm" onClick={() => setFilters(s.globalFilters)}>
              ביטול שינויים
            </Button>
            <Button
              size="sm"
              variant="primary"
              onClick={() =>
                update({ globalFilters: filters })
                  .then(() => toast({ tone: "ok", title: "כללי הסינון נשמרו" }))
                  .catch(() => {})
              }
            >
              שמירת כללים
            </Button>
          </div>
        )}
      </Card>

      <TaskListBackupCard snap={snap} refresh={refresh} update={update} />

      <Card title="התראות" icon={<Bell size={18} />}>
        <Toggle checked={s.notifySuccess} onChange={(notifySuccess) => update({ notifySuccess })} label="התראה כשגיבוי מצליח" />
        <Toggle checked={s.notifyFailure} onChange={(notifyFailure) => update({ notifyFailure })} label="התראה כשגיבוי נכשל" />
        <SoundPicker
          label="צליל להצלחה"
          hint="גם כשגיבוי מתחיל מעצמו בחיבור כונן."
          value={s.soundSuccess}
          onChange={(soundSuccess) => update({ soundSuccess })}
        />
        <SoundPicker
          label="צליל לכישלון ולאזהרות"
          value={s.soundFailure}
          onChange={(soundFailure) => update({ soundFailure })}
        />
        <p className="text-xs text-muted">
          הצלילים הם של Windows. אם לא נשמע כלום, בדקו שההתראות של Backuper והצלילים שלהן מופעלים בהגדרות Windows - מערכת - הודעות, ושמצב &quot;נא לא להפריע&quot; כבוי.
        </p>
      </Card>

      <p className="text-center text-xs text-muted">Backuper {version} - מנוע העתקה: robocopy</p>
    </div>
  );
}

/** Windows toast sounds (the names its notification XML accepts). */
const SOUNDS = [
  { value: "", label: "ללא צליל" },
  { value: "Default", label: "רגיל" },
  { value: "Reminder", label: "תזכורת" },
  { value: "Mail", label: "דואר" },
  { value: "IM", label: "הודעה" },
  { value: "SMS", label: "SMS" },
];

/** A notification sound choice, with a button that plays it as a sample notification. */
function SoundPicker({
  label,
  hint,
  value,
  onChange,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (sound: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex flex-col gap-0.5">
        <span className="text-sm font-medium">{label}</span>
        {hint && <span className="text-xs text-muted">{hint}</span>}
      </div>
      <div className="flex items-center gap-1.5">
        <Select className="w-40" value={value} onChange={(e) => onChange(e.target.value)} aria-label={label}>
          {SOUNDS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </Select>
        <IconButton label="השמעה לדוגמה" onClick={() => api.testSound(value).catch(() => {})} disabled={!value}>
          <Volume2 size={16} />
        </IconButton>
      </div>
    </div>
  );
}

const SHOWN_SNAPSHOTS = 5;

/** Snapshots of the task list, newest first, each with a restore button. */
function SnapshotList({
  snapshots,
  onRestore,
  markCurrent,
}: {
  snapshots: TaskSnapshot[];
  onRestore: (s: TaskSnapshot) => void;
  /** The newest one is the current list (the automatic snapshots). */
  markCurrent: boolean;
}) {
  const [showAll, setShowAll] = useState(false);
  const shown = showAll ? snapshots : snapshots.slice(0, SHOWN_SNAPSHOTS);
  return (
    <>
      <ul className="flex flex-col divide-y divide-line rounded-lg border border-line">
        {shown.map((s, i) => (
          <li key={s.path} className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-[13px]">
            <span className="font-medium">{fmtSmart(s.savedAt)}</span>
            <span className="text-muted">{fmtRelative(s.savedAt)}</span>
            <span className="text-muted">· {s.taskCount === 1 ? "משימה אחת" : `${s.taskCount} משימות`}</span>
            {markCurrent && i === 0 && <span className="text-xs text-muted">(המצב הנוכחי)</span>}
            <Button size="sm" variant="ghost" className="ms-auto" icon={<RotateCcw size={14} />} onClick={() => onRestore(s)}>
              שחזור
            </Button>
          </li>
        ))}
      </ul>
      {snapshots.length > SHOWN_SNAPSHOTS && (
        <button type="button" className="self-start text-[13px] text-accent hover:underline" onClick={() => setShowAll(!showAll)}>
          {showAll ? "הצגת פחות" : `הצגת כל ${snapshots.length} העותקים`}
        </button>
      )}
    </>
  );
}

/** Automatic snapshots of the task list (restore), snapshots in a folder of choice, export/import. */
function TaskListBackupCard({
  snap,
  refresh,
  update,
}: {
  snap: Snapshot;
  refresh: () => void;
  update: (patch: Partial<Settings>) => Promise<void>;
}) {
  const { toast } = useFeedback();
  const transfer = useTaskListTransfer(snap.tasks, refresh);
  const [snapshots, setSnapshots] = useState<TaskSnapshot[]>([]);
  // Snapshots found in a folder the user picked (e.g. the backup drive after reinstalling).
  const [other, setOther] = useState<{ dir: string; snapshots: TaskSnapshot[] } | null>(null);
  // A new snapshot appears whenever the tasks change.
  useEffect(() => {
    api.listTaskSnapshots().then(setSnapshots).catch(() => setSnapshots([]));
  }, [snap.tasks]);

  const saved = snap.settings.taskListCopyDir;
  const [dir, setDir] = useState(saved);
  useEffect(() => setDir(saved), [saved]);
  const saveDir = (taskListCopyDir: string) =>
    update({ taskListCopyDir })
      .then(() =>
        toast(
          taskListCopyDir
            ? { tone: "ok", title: "רשימת המשימות נשמרה בתיקייה", message: <PathText path={taskListCopyDir} /> }
            : { tone: "info", title: "שמירת העותק בתיקייה הופסקה" },
        ),
      )
      .catch(() => {});

  const restore = (s: TaskSnapshot) => {
    setOther(null);
    transfer.importSnapshot(s.path, fmtSmart(s.savedAt));
  };
  const restoreFromFolder = async () => {
    const picked = await api.pickFolder("בחירת התיקייה שבה נשמרה רשימת המשימות", saved);
    if (!picked) return;
    const found = await api.listTaskSnapshots(picked).catch(() => []);
    if (found.length === 0) {
      toast({
        tone: "bad",
        title: "לא נמצאו בתיקייה גיבויים של רשימת המשימות",
        message: "בחרו את התיקייה שהוגדרה כאן לשמירת עותקים (למשל בכונן הגיבוי).",
      });
    } else {
      setOther({ dir: picked, snapshots: found });
    }
  };

  return (
    <Card title="גיבוי רשימת המשימות" icon={<History size={18} />}>
      <p className="-mt-2 text-[13px] text-muted">
        בכל שינוי ברשימת המשימות נשמר עותק שלה (50 העותקים האחרונים), ואפשר לשחזר ממנו משימות שנמחקו או שונו. העותקים
        מוסתרים ומוגנים מפני מחיקה בטעות, והשחזור נעשה מכאן.
      </p>

      <div className="flex flex-col gap-2">
        <span className="text-sm font-medium">עותקים גם בתיקייה נוספת</span>
        <span className="-mt-1 text-xs text-muted">
          למשל בכונן הגיבוי, כדי שהרשימה תישמר גם אם המחשב עצמו נפגע. בכל שינוי נשמר שם עותק מתוארך (50 האחרונים),
          בתיקייה מוסתרת. אחרי התקנה מחדש משחזרים ממנה עם "שחזור מתיקייה".
        </span>
        <PathPicker value={dir} onChange={setDir} title="תיקייה לעותק של רשימת המשימות" placeholder="K:\Backuper" />
        {(dir.trim() !== saved || saved) && (
          <div className="flex justify-end gap-2">
            {saved && dir.trim() === saved && (
              <Button size="sm" onClick={() => saveDir("")}>
                הפסקת השמירה בתיקייה
              </Button>
            )}
            {dir.trim() !== saved && (
              <Button size="sm" variant="primary" onClick={() => saveDir(dir.trim())}>
                {dir.trim() ? "שמירה" : "הפסקת השמירה בתיקייה"}
              </Button>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-sm font-medium">עותקים אוטומטיים</span>
        {snapshots.length === 0 ? (
          <span className="text-[13px] text-muted">עדיין אין עותקים - הראשון נשמר בשינוי הבא ברשימה.</span>
        ) : (
          <SnapshotList snapshots={snapshots} onRestore={restore} markCurrent />
        )}
      </div>

      <div className="flex flex-wrap gap-2">
        <Button icon={<Download size={16} />} disabled={snap.tasks.length === 0} onClick={() => transfer.exportTasks()}>
          ייצוא כל המשימות לקובץ
        </Button>
        <Button icon={<Import size={16} />} onClick={transfer.importFile}>
          ייבוא מקובץ (Backuper או Cobian)
        </Button>
        <Button icon={<FolderSearch size={16} />} onClick={restoreFromFolder}>
          שחזור מתיקייה
        </Button>
      </div>
      {other && (
        <Modal title="שחזור רשימת המשימות" subtitle={<PathText path={other.dir} />} onClose={() => setOther(null)}>
          <div className="flex flex-col gap-2">
            <SnapshotList snapshots={other.snapshots} onRestore={restore} markCurrent={false} />
          </div>
        </Modal>
      )}
      {transfer.dialog}
    </Card>
  );
}
