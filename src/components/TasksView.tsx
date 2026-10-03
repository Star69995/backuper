import {
  AlertTriangle,
  ArrowDownAZ,
  ArrowDownUp,
  ArrowLeft,
  ArrowUpZA,
  CheckCircle2,
  ChevronDown,
  CircleSlash,
  Clock,
  Copy,
  Download,
  Folder,
  FolderOpen,
  FolderTree,
  HardDrive,
  Import,
  ListChecks,
  MoreVertical,
  Pencil,
  Play,
  Plus,
  Power,
  PowerOff,
  Search,
  Square,
  Trash2,
  X,
  XCircle,
} from "lucide-react";
import { Fragment, useMemo, useRef, useState } from "react";
import { api, errorText } from "../api";
import { describeSchedule, fmtRelative, fmtSmart, KIND_LABEL, MODE_LABEL, nextRunOf, taskKind } from "../lib/format";
import { useColumnWidths } from "../lib/useColumnWidths";
import { groupTasks, SORT_LABEL, type SortKey, sortTasks, useTaskSort } from "../lib/sortTasks";
import type { BackupMode, RunRecord, RunStatus, Snapshot, Task } from "../types";
import { newTask } from "../types";
import BackupsDialog from "./BackupsDialog";
import BulkEditDialog from "./BulkEditDialog";
import { RunDetails } from "./HistoryView";
import { useFeedback } from "./feedback";
import { useTaskListTransfer } from "./ImportTasksDialog";
import RunningCard from "./RunningCard";
import TaskEditor from "./TaskEditor";
import { Badge, Button, Checkbox, ColumnResizer, cx, EmptyState, IconButton, Menu, type MenuItem, PathLink, PathText, Select, Spinner, TextInput } from "./ui";

export const statusBadge = (s: RunStatus) =>
  ({
    success: (
      <Badge tone="ok" icon={<CheckCircle2 size={12} />}>
        הצליח
      </Badge>
    ),
    warning: (
      <Badge tone="warn" icon={<AlertTriangle size={12} />}>
        אזהרות
      </Badge>
    ),
    failed: (
      <Badge tone="bad" icon={<XCircle size={12} />}>
        נכשל
      </Badge>
    ),
    cancelled: <Badge icon={<CircleSlash size={12} />}>בוטל</Badge>,
  })[s];

type ColumnKey = "name" | "kind" | "schedule" | "next" | "last";
const RESIZABLE: ColumnKey[] = ["name", "kind", "schedule", "next", "last"];
const COLUMN_LABEL: Record<ColumnKey, string> = {
  name: "משימה",
  kind: "סוג",
  schedule: "תזמון",
  next: "ריצה הבאה",
  last: "ריצה אחרונה",
};
/** name 0 = auto: fills the free space until the user resizes any column. */
const DEFAULT_WIDTHS: Record<ColumnKey, number> = { name: 0, kind: 112, schedule: 176, next: 128, last: 128 };
const MIN_NAME_COL = 220;
const CHECK_COL = 40;
/** The actions column also takes any leftover width, so a dragged border follows the mouse exactly. */
const MIN_ACTIONS_COL = 96;

/** "Run full now" / "Run incremental now" menu entries. */
const runModeItems = (run: (mode: BackupMode) => void): MenuItem[] => [
  { label: "הרץ גיבוי מלא עכשיו", icon: <Play size={14} />, onClick: () => run("full") },
  { label: "הרץ גיבוי אינקרמנטלי עכשיו", icon: <Play size={14} />, onClick: () => run("incremental") },
];

export default function TasksView({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast, confirm } = useFeedback();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<Task | null>(null);
  const [backupsOf, setBackupsOf] = useState<Task | null>(null);
  const [bulk, setBulk] = useState<Task[] | null>(null);
  const [runDetails, setRunDetails] = useState<RunRecord | null>(null);
  // Tasks whose source list is expanded in the table (multi-source tasks only).
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const toggleSources = (id: string) =>
    setExpanded((s) => {
      const n = new Set(s);
      if (!n.delete(id)) n.add(id);
      return n;
    });

  const [sort, setSort] = useTaskSort();
  const cols = useColumnWidths("backuper.taskColumns", DEFAULT_WIDTHS);
  // Wider than the view: the table scrolls sideways.
  const nameAuto = cols.widths.name === 0;
  const tableMinWidth =
    CHECK_COL + MIN_ACTIONS_COL + (nameAuto ? MIN_NAME_COL : 0) + RESIZABLE.reduce((sum, k) => sum + cols.widths[k], 0);
  const nameHeader = useRef<HTMLTableCellElement>(null);
  // Once a border is dragged, every column gets a fixed width, so only the dragged border moves.
  const freezeName = () => nameAuto && nameHeader.current && cols.setWidth("name", nameHeader.current.getBoundingClientRect().width);
  const transfer = useTaskListTransfer(snap.tasks, refresh);
  const tasks = useMemo(() => {
    const q = query.trim().toLowerCase();
    const found = q
      ? snap.tasks.filter((t) =>
          [t.name, t.destination, ...t.sources.map((s) => s.path)].some((s) => s.toLowerCase().includes(q)),
        )
      : snap.tasks;
    return sortTasks(found, sort, snap.states);
  }, [snap.tasks, snap.states, query, sort]);
  const groups = useMemo(() => groupTasks(tasks, sort.key), [tasks, sort.key]);

  // Drop selections of tasks that no longer exist.
  const sel = [...selected].filter((id) => snap.tasks.some((t) => t.id === id));
  const selTasks = snap.tasks.filter((t) => sel.includes(t.id));
  const allChecked = tasks.length > 0 && tasks.every((t) => selected.has(t.id));
  const someChecked = tasks.some((t) => selected.has(t.id));

  const toggle = (id: string, on: boolean) =>
    setSelected((s) => {
      const n = new Set(s);
      if (on) n.add(id);
      else n.delete(id);
      return n;
    });

  const run = async (ids: string[], mode: BackupMode | null = null) => {
    const n = await api.runTasks(ids, mode);
    if (n === 0) toast({ tone: "info", title: "המשימה כבר רצה או ממתינה בתור" });
    else toast({ tone: "info", title: n === 1 ? "הגיבוי נוסף לתור" : `${n} גיבויים נוספו לתור` });
  };

  /** Saves tasks and offers to undo by restoring the previous versions. */
  const saveWithUndo = async (next: Task[], before: Task[], title: string) => {
    try {
      await api.saveTasks(next);
      refresh();
      toast({
        tone: "ok",
        title,
        action: {
          label: "ביטול",
          onClick: () =>
            api
              .saveTasks(before)
              .then(() => toast({ tone: "info", title: "השינוי בוטל" }))
              .catch((e) => toast({ tone: "bad", title: "הביטול נכשל", message: errorText(e) })),
        },
      });
    } catch (e) {
      toast({ tone: "bad", title: "השמירה נכשלה", message: errorText(e) });
    }
  };

  /** The task's newest run record (history is newest first), shown in the run details dialog. */
  const showLastRun = async (taskId: string) => {
    const run = (await api.history()).find((r) => r.taskId === taskId);
    if (run) setRunDetails(run);
    else toast({ tone: "info", title: "פרטי הריצה כבר לא נמצאים ביומן", message: "ייתכן שהיומן נוקה." });
  };

  const setEnabled = (list: Task[], enabled: boolean) =>
    saveWithUndo(
      list.map((t) => ({ ...t, enabled })),
      list,
      enabled ? `${list.length} משימות הופעלו` : `${list.length} משימות הושבתו`,
    );

  const remove = async (list: Task[]) => {
    const ok = await confirm({
      title: list.length === 1 ? `למחוק את המשימה "${list[0].name}"?` : `למחוק ${list.length} משימות?`,
      message: "המשימות יוסרו מהתוכנה. קבצי הגיבוי שכבר נוצרו ביעד לא יימחקו.",
      confirmLabel: "מחק",
      danger: true,
    });
    if (!ok) return;
    await api.deleteTasks(list.map((t) => t.id));
    setSelected(new Set());
    refresh();
    toast({
      tone: "ok",
      title: list.length === 1 ? "המשימה נמחקה" : `${list.length} משימות נמחקו`,
      action: { label: "ביטול", onClick: () => api.saveTasks(list).then(refresh) },
    });
  };

  // Copies get their own folder names, so they don't share (and delete) the original's backups.
  const duplicate = (t: Task) =>
    setEditing({
      ...t,
      id: "",
      name: `${t.name} (עותק)`,
      sources: t.sources.map((s) => ({ ...s, folderName: `${s.folderName} (עותק)` })),
    });

  const queuedIds = new Set(snap.queue.map((j) => j.taskId));
  const runningId = snap.current?.taskId;

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-4 p-4 md:p-6">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold">משימות גיבוי</h1>
          <p className="text-[13px] text-muted">
            {snap.tasks.length === 0
              ? "עדיין לא הוגדרו משימות"
              : `${snap.tasks.length} משימות, ${snap.tasks.filter((t) => t.enabled).length} פעילות`}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {snap.tasks.length > 1 && (
            <div className="flex items-center gap-1">
              <Select
                aria-label="מיון"
                className="w-48"
                value={sort.key}
                onChange={(e) => setSort({ ...sort, key: e.target.value as SortKey })}
              >
                {(Object.keys(SORT_LABEL) as SortKey[]).map((k) => (
                  <option key={k} value={k}>
                    מיון: {SORT_LABEL[k]}
                  </option>
                ))}
              </Select>
              <IconButton
                label={sort.desc ? "סדר יורד - לחצו לסדר עולה" : "סדר עולה - לחצו לסדר יורד"}
                onClick={() => setSort({ ...sort, desc: !sort.desc })}
                className="size-9 border border-line bg-panel"
              >
                {sort.desc ? <ArrowUpZA size={16} /> : <ArrowDownAZ size={16} />}
              </IconButton>
            </div>
          )}
          {snap.tasks.length > 0 && (
            <div className="relative">
              <Search size={16} className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-muted" />
              <TextInput
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="חיפוש..."
                className="w-44 ps-9 sm:w-56"
              />
            </div>
          )}
          <Menu
            trigger={(open) => (
              <Button icon={<ArrowDownUp size={16} />} onClick={open}>
                ייבוא וייצוא
              </Button>
            )}
            items={[
              { label: "ייבוא משימות מקובץ (Backuper או Cobian)", icon: <Import size={14} />, onClick: transfer.importFile },
              {
                label: "ייצוא כל המשימות לקובץ",
                icon: <Download size={14} />,
                disabled: snap.tasks.length === 0,
                onClick: () => transfer.exportTasks(),
              },
            ]}
          />
          <Button variant="primary" icon={<Plus size={16} />} onClick={() => setEditing(newTask())}>
            משימה חדשה
          </Button>
        </div>
      </header>

      {snap.current && <RunningCard progress={snap.current} queue={snap.queue} tasks={snap.tasks} />}

      {sel.length > 0 && (
        <div className="sticky top-0 z-10 flex flex-wrap items-center gap-2 rounded-xl border border-accent/30 bg-accent-soft px-3 py-2 shadow-sm">
          <span className="text-sm font-medium text-accent">נבחרו {sel.length}</span>
          <div className="mx-1 h-5 w-px bg-accent/20" />
          {selTasks.some((t) => taskKind(t) === "combined") ? (
            <Menu
              trigger={(open) => (
                <Button size="sm" variant="ghost" icon={<Play size={15} />} onClick={open}>
                  הרץ
                </Button>
              )}
              items={[
                { label: "לפי סוג כל משימה", icon: <Play size={14} />, onClick: () => run(sel) },
                ...runModeItems((mode) => run(sel, mode)),
              ]}
            />
          ) : (
            <Button size="sm" variant="ghost" icon={<Play size={15} />} onClick={() => run(sel)}>
              הרץ
            </Button>
          )}
          <Button size="sm" variant="ghost" icon={<ListChecks size={15} />} onClick={() => setBulk(selTasks)}>
            עריכה מרובה
          </Button>
          <Button size="sm" variant="ghost" icon={<Download size={15} />} onClick={() => transfer.exportTasks(selTasks)}>
            ייצוא
          </Button>
          <Button size="sm" variant="ghost" icon={<Power size={15} />} onClick={() => setEnabled(selTasks, true)}>
            הפעל
          </Button>
          <Button size="sm" variant="ghost" icon={<PowerOff size={15} />} onClick={() => setEnabled(selTasks, false)}>
            השבת
          </Button>
          <Button size="sm" variant="ghost" className="text-bad" icon={<Trash2 size={15} />} onClick={() => remove(selTasks)}>
            מחק
          </Button>
          <IconButton label="ניקוי בחירה" className="ms-auto" onClick={() => setSelected(new Set())}>
            <X size={16} />
          </IconButton>
        </div>
      )}

      <section className="overflow-hidden rounded-xl border border-line bg-panel">
        {snap.tasks.length === 0 ? (
          <EmptyState icon={<HardDrive size={26} />} title="בואו ניצור את משימת הגיבוי הראשונה">
            <p>בחרו תיקיית מקור, יעד ולוח זמנים. הגיבוי נשמר כקבצים רגילים שאפשר לפתוח ישירות בסייר הקבצים.</p>
            <div className="mt-4 flex flex-wrap justify-center gap-2">
              <Button variant="primary" icon={<Plus size={16} />} onClick={() => setEditing(newTask())}>
                משימה חדשה
              </Button>
              <Button icon={<Import size={16} />} onClick={transfer.importFile}>
                ייבוא מקובץ
              </Button>
            </div>
          </EmptyState>
        ) : tasks.length === 0 ? (
          <div className="p-8 text-center text-sm text-muted">אין משימות שתואמות לחיפוש</div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full table-fixed text-sm" style={{ minWidth: tableMinWidth }}>
              <colgroup>
                <col style={{ width: CHECK_COL }} />
                {RESIZABLE.map((k) => (
                  <col key={k} style={{ width: cols.widths[k] || undefined }} />
                ))}
                <col style={{ width: nameAuto ? MIN_ACTIONS_COL : undefined }} />
              </colgroup>
              <thead>
                <tr className="border-b border-line bg-panel2 text-xs text-muted">
                  <th className="px-3 py-2.5">
                    <Checkbox
                      label="בחר הכול"
                      checked={allChecked}
                      indeterminate={someChecked && !allChecked}
                      onChange={(on) =>
                        setSelected((s) => {
                          const n = new Set(s);
                          tasks.forEach((t) => (on ? n.add(t.id) : n.delete(t.id)));
                          return n;
                        })
                      }
                    />
                  </th>
                  {RESIZABLE.map((k) => (
                    <th
                      key={k}
                      ref={k === "name" ? nameHeader : undefined}
                      className="relative px-2 py-2.5 text-start font-medium"
                    >
                      {COLUMN_LABEL[k]}
                      <ColumnResizer
                        label={`רוחב העמודה ${COLUMN_LABEL[k]}`}
                        onStart={freezeName}
                        onResize={(w) => cols.setWidth(k, w)}
                        onReset={() => cols.reset(k)}
                      />
                    </th>
                  ))}
                  <th className="px-2 py-2.5" />
                </tr>
              </thead>
              <tbody>
                {groups.map((g, gi) => (
                  <Fragment key={g.folder ?? gi}>
                    {g.folder !== null && (
                      <tr className="border-b border-line bg-panel2/70">
                        <td colSpan={7} className="px-3 py-1.5">
                          <div className="flex items-center gap-2 text-xs font-medium text-muted">
                            <Folder size={14} className="shrink-0 text-accent" />
                            <span className="text-fg">{sort.key === "source" ? "מקור:" : "יעד:"}</span>
                            <PathText path={g.folder} className="min-w-0 truncate text-fg" />
                            <span>({g.tasks.length})</span>
                            <IconButton label="פתיחת התיקייה" className="ms-auto size-6" onClick={() => openPath(g.folder!)}>
                              <FolderOpen size={14} />
                            </IconButton>
                          </div>
                        </td>
                      </tr>
                    )}
                    {g.tasks.map((t) => {
                      const st = snap.states[t.id];
                      const next = nextRunOf(t, st);
                      const running = runningId === t.id;
                      const queued = queuedIds.has(t.id);
                      const checked = selected.has(t.id);
                      return (
                        <tr
                          key={t.id}
                          onDoubleClick={() => setEditing(t)}
                          className={cx(
                            "group cursor-default border-b border-line last:border-b-0 transition-colors",
                            checked ? "bg-accent-soft/60" : "hover:bg-hover/60",
                            !t.enabled && "text-muted",
                          )}
                        >
                          <td
                            className="cursor-pointer px-3 py-3"
                            onClick={() => toggle(t.id, !checked)}
                            onDoubleClick={(e) => e.stopPropagation()}
                          >
                            <Checkbox label={`בחר ${t.name}`} checked={checked} onChange={(on) => toggle(t.id, on)} />
                          </td>
                          <td className="px-2 py-3">
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                setEditing(t);
                              }}
                              className="flex max-w-full items-center gap-2 text-start font-medium text-fg hover:text-accent"
                            >
                              <span className="truncate">{t.name}</span>
                              {!t.enabled && <Badge>מושבת</Badge>}
                            </button>
                            <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-muted">
                              {t.sources.length === 1 ? (
                                <PathLink path={t.sources[0].path} onOpen={openPath} />
                              ) : (
                                <button
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    toggleSources(t.id);
                                  }}
                                  onDoubleClick={(e) => e.stopPropagation()}
                                  aria-expanded={expanded.has(t.id)}
                                  className="flex shrink-0 items-center gap-0.5 rounded hover:text-accent"
                                  title={expanded.has(t.id) ? "הסתרת תיקיות המקור" : t.sources.map((s) => s.path).join("\n")}
                                >
                                  <ChevronDown
                                    size={13}
                                    className={cx("transition-transform", !expanded.has(t.id) && "rotate-90")}
                                  />
                                  {t.sources.length} תיקיות מקור
                                </button>
                              )}
                              <ArrowLeft size={12} className="shrink-0" />
                              <PathLink path={t.destination} onOpen={openPath} />
                            </div>
                            {t.sources.length > 1 && expanded.has(t.id) && (
                              <ul className="ms-4 mt-1 flex flex-col gap-0.5 border-s border-line ps-2 text-xs text-muted">
                                {t.sources.map((s, i) => (
                                  <li key={i} className="flex min-w-0">
                                    <PathLink path={s.path} onOpen={openPath} />
                                  </li>
                                ))}
                              </ul>
                            )}
                          </td>
                          <td className="px-2 py-3">
                            <Badge tone={taskKind(t) === "incremental" ? "neutral" : "accent"}>{KIND_LABEL[taskKind(t)]}</Badge>
                          </td>
                          <td className="px-2 py-3 text-[13px]">
                            {t.fullSchedule && t.mode === "incremental" ? (
                              <div className="flex flex-col gap-0.5">
                                <span>
                                  <span className="text-muted">אינקרמנטלי: </span>
                                  {describeSchedule(t.schedule)}
                                </span>
                                <span>
                                  <span className="text-muted">מלא: </span>
                                  {describeSchedule(t.fullSchedule)}
                                </span>
                              </div>
                            ) : (
                              describeSchedule(t.schedule)
                            )}
                          </td>
                          <td className="px-2 py-3 text-[13px]">
                            {!next ? (
                              <span className="text-muted">-</span>
                            ) : (
                              <div className={cx(snap.settings.schedulerPaused && "text-muted line-through")}>
                                <div>{fmtSmart(next.at)}</div>
                                <div className="text-xs text-muted">
                                  {fmtRelative(next.at)}
                                  {t.fullSchedule && ` (${MODE_LABEL[next.mode]})`}
                                </div>
                              </div>
                            )}
                          </td>
                          <td className="px-2 py-3 text-[13px]">
                            {running ? (
                              <Badge tone="accent" icon={<Spinner className="size-3 border-[1.5px]" />}>
                                רץ כעת
                              </Badge>
                            ) : queued ? (
                              <Badge tone="accent" icon={<Clock size={12} />}>
                                ממתין בתור
                              </Badge>
                            ) : st?.lastStatus ? (
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  showLastRun(t.id);
                                }}
                                className="-m-1 flex flex-col items-start gap-1 rounded-lg p-1 text-start hover:bg-hover"
                                title={`${st.lastMessage ?? ""}\nלחיצה לפרטי הריצה וליומן המלא`}
                              >
                                {statusBadge(st.lastStatus)}
                                <span className="text-xs text-muted">{fmtSmart(st.lastRunAt)}</span>
                              </button>
                            ) : (
                              <span className="text-muted">טרם רץ</span>
                            )}
                          </td>
                          <td className="px-2 py-3" onClick={(e) => e.stopPropagation()}>
                            <div className="flex items-center justify-end gap-0.5">
                              {running || queued ? (
                                <IconButton label="עצור" tone="danger" onClick={() => api.cancelTask(t.id)}>
                                  <Square size={15} />
                                </IconButton>
                              ) : taskKind(t) === "combined" ? (
                                <Menu
                                  trigger={(open) => (
                                    <IconButton label="הרץ עכשיו" onClick={open}>
                                      <Play size={16} />
                                    </IconButton>
                                  )}
                                  items={runModeItems((mode) => run([t.id], mode))}
                                />
                              ) : (
                                <IconButton label="הרץ עכשיו" onClick={() => run([t.id])}>
                                  <Play size={16} />
                                </IconButton>
                              )}
                              <Menu
                                trigger={(open) => (
                                  <IconButton label="פעולות נוספות" onClick={open}>
                                    <MoreVertical size={16} />
                                  </IconButton>
                                )}
                                items={[
                                  ...runModeItems((mode) => run([t.id], mode)),
                                  "separator",
                                  { label: "עריכה", icon: <Pencil size={14} />, onClick: () => setEditing(t) },
                                  { label: "שכפול", icon: <Copy size={14} />, onClick: () => duplicate(t) },
                                  {
                                    label: t.enabled ? "השבתה" : "הפעלה",
                                    icon: t.enabled ? <PowerOff size={14} /> : <Power size={14} />,
                                    onClick: () => setEnabled([t], !t.enabled),
                                  },
                                  "separator",
                                  { label: "ניהול גיבויים", icon: <FolderTree size={14} />, onClick: () => setBackupsOf(t) },
                                  {
                                    label: "פתיחת תיקיית היעד",
                                    icon: <FolderOpen size={14} />,
                                    onClick: () => openPath(t.destination),
                                  },
                                  ...t.sources.map((s) => ({
                                    label: t.sources.length === 1 ? "פתיחת תיקיית המקור" : `פתיחת מקור: ${s.folderName}`,
                                    icon: <FolderOpen size={14} />,
                                    onClick: () => openPath(s.path),
                                  })),
                                  "separator",
                                  { label: "מחיקה", icon: <Trash2 size={14} />, danger: true, onClick: () => remove([t]) },
                                ]}
                              />
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                  </Fragment>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      {snap.tasks.length > 0 && <p className="text-xs text-muted">טיפ: לחיצה על שורה בוחרת אותה, לחיצה כפולה פותחת עריכה.</p>}

      {editing && (
        <TaskEditor
          task={editing}
          globalFilterCount={snap.settings.globalFilters.length}
          onClose={() => setEditing(null)}
          onSaved={(saved, isNew) => {
            setEditing(null);
            refresh();
            toast({ tone: "ok", title: isNew ? `המשימה "${saved.name}" נוצרה` : "השינויים נשמרו" });
          }}
        />
      )}
      {transfer.dialog}
      {runDetails && <RunDetails run={runDetails} onClose={() => setRunDetails(null)} />}
      {backupsOf && <BackupsDialog task={backupsOf} onClose={() => setBackupsOf(null)} />}
      {bulk && (
        <BulkEditDialog
          tasks={bulk}
          onClose={() => setBulk(null)}
          onApply={async (next) => {
            setBulk(null);
            await saveWithUndo(next, bulk, `${next.length} משימות עודכנו`);
          }}
        />
      )}
    </div>
  );

  function openPath(p: string) {
    api.openPath(p).catch((e) => toast({ tone: "bad", title: "לא ניתן לפתוח את התיקייה", message: errorText(e) }));
  }
}
