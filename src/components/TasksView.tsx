import {
  AlertTriangle,
  Archive,
  ArrowDownAZ,
  ArrowDown,
  ArrowDownUp,
  ArrowLeft,
  ArrowUp,
  ArrowUpZA,
  CheckCircle2,
  ChevronDown,
  CircleSlash,
  Clock,
  Copy,
  Diff,
  Download,
  Folder,
  FolderOpen,
  FolderInput,
  FolderPlus,
  FolderTree,
  GripVertical,
  History,
  HardDrive,
  Import,
  Layers,
  ListChecks,
  Minimize2,
  MoreVertical,
  Pencil,
  Plus,
  Power,
  PowerOff,
  RotateCcw,
  Pause,
  Square,
  Trash2,
  Undo2,
  X,
  XCircle,
} from "lucide-react";
import { Fragment, type PointerEvent as ReactPointerEvent, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { api, errorText } from "../api";
import {
  describeSchedule,
  fmtBytes,
  fmtRelative,
  fmtSmart,
  KIND_LABEL,
  MODE_LABEL,
  nextRunOf,
  taskKind,
  type TaskKind,
} from "../lib/format";
import { fitColumns, useColumnWidths } from "../lib/useColumnWidths";
import { useDiskSpace, useFullSpace } from "../lib/useDiskSpace";
import { groupTasks, moveInOrder, SORT_LABEL, type SortKey, sortTasks, useCustomOrder, useTaskSort } from "../lib/sortTasks";
import type { BackupMode, DiskSpace, FullSpace, LastBackup, RunRecord, RunStatus, Snapshot, Task } from "../types";
import { newTask } from "../types";
import BackupsDialog from "./BackupsDialog";
import BulkEditDialog from "./BulkEditDialog";
import { CobianPendingDialog, CobianRestoreDialog, CobianTakeoverDialog } from "./CobianDialogs";
import { GENERAL_GROUP, groupNames, GroupNameDialog, groupOf, sameGroup } from "./groups";
import { RunDetails } from "./HistoryView";
import { useFeedback } from "./feedback";
import { useResumeTask } from "./resumeTask";
import { useStopTask } from "./stopTask";
import { useTaskListTransfer } from "./ImportTasksDialog";
import PausedCard from "./PausedCard";
import RunningCard from "./RunningCard";
import TaskEditor from "./TaskEditor";
import {
  Badge,
  Button,
  Checkbox,
  ColumnResizer,
  cx,
  EmptyState,
  IconButton,
  Menu,
  type MenuItem,
  PathLink,
  PathText,
  RunIcon,
  SearchBox,
  Select,
  Spinner,
  Toggle,
} from "./ui";

/** `resumable` (only matters for a stopped run): paused with a folder to continue in, or stopped for good. */
export const statusBadge = (s: RunStatus, resumable = false) =>
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
    cancelled: resumable ? (
      <Badge tone="warn" icon={<Pause size={12} />}>
        מושהה
      </Badge>
    ) : (
      <Badge icon={<CircleSlash size={12} />}>הופסק</Badge>
    ),
  })[s];

type ColumnKey = "name" | "enabled" | "kind" | "schedule" | "next" | "last";
const RESIZABLE: ColumnKey[] = ["name", "enabled", "kind", "schedule", "next", "last"];
const COLUMN_LABEL: Record<ColumnKey, string> = {
  name: "משימה",
  enabled: "מצב",
  kind: "סוג",
  schedule: "תזמון",
  next: "ריצה הבאה",
  last: "ריצה אחרונה",
};
/** name 0 = auto: fills the free space until the user resizes any column. */
const DEFAULT_WIDTHS: Record<ColumnKey, number> = { name: 0, enabled: 120, kind: 112, schedule: 176, next: 128, last: 128 };
const MIN_NAME_COL = 220;
/** How narrow columns may get when the window is too small for their wanted widths. */
const MIN_NAME_FLOOR = 140;
const MIN_COL_FLOOR = 56;
/** Columns with badges / a toggle + label never wrap their content, so they may not get narrower than it ("ריצה אחרונה": "ממתין בתור"). */
const COL_FLOORS: Record<ColumnKey, number> = {
  name: MIN_NAME_FLOOR,
  enabled: 100,
  kind: 80,
  schedule: MIN_COL_FLOOR,
  next: MIN_COL_FLOOR,
  last: 112,
};
/** Columns dropped, in this order, when the window is too narrow for all of them (the kind shows as an icon next to the task name). */
const NARROW_DROP: ColumnKey[] = ["kind", "schedule"];
const CHECK_COL = 40;
/** Custom order: the checkbox column also holds the drag handle. */
const CHECK_COL_CUSTOM = 64;
/** Compact view: "מצב" and "סוג" show only a toggle / an icon, so they get a fixed narrow width. */
const COMPACT_COLS: Partial<Record<ColumnKey, number>> = { enabled: 64, kind: 64 };
const KIND_ICON: Record<TaskKind, ReactNode> = {
  full: <Archive size={14} />,
  incremental: <Diff size={14} />,
  combined: <Layers size={14} />,
};

const COMPACT_KEY = "backuper.taskCompact";

/** Compact table view (no text next to the status toggle / kind), remembered per machine. */
function useCompactView() {
  const [compact, setCompact] = useState(() => {
    try {
      return localStorage.getItem(COMPACT_KEY) === "1";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(COMPACT_KEY, compact ? "1" : "0");
    } catch {
      /* ignore */
    }
  }, [compact]);
  return [compact, setCompact] as const;
}
/** The actions column also takes any leftover width, so a dragged border follows the mouse exactly. */
/** Room for "המשך" (a paused backup) + the menu. */
const MIN_ACTIONS_COL = 112;

/** "Run full now" / "Run incremental now" menu entries. */
const runModeItems = (run: (mode: BackupMode) => void): MenuItem[] => [
  { label: "הרצת גיבוי מלא עכשיו", icon: <RunIcon size={14} />, onClick: () => run("full") },
  { label: "הרצת גיבוי אינקרמנטלי עכשיו", icon: <RunIcon size={14} />, onClick: () => run("incremental") },
];

const GROUP_KEY = "backuper.taskGroup";

/** The group tab shown: null = all tasks, "" = tasks without a group. Remembered per machine. */
function useActiveGroup() {
  const [group, setGroup] = useState<string | null>(() => {
    try {
      const v = JSON.parse(localStorage.getItem(GROUP_KEY) ?? "null");
      return typeof v === "string" ? v : null;
    } catch {
      return null;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(GROUP_KEY, JSON.stringify(group));
    } catch {
      /* ignore */
    }
  }, [group]);
  return [group, setGroup] as const;
}

export default function TasksView({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast, confirm, choose } = useFeedback();
  const stopTask = useStopTask();
  const [activeGroup, setActiveGroup] = useActiveGroup();
  // Groups just created here that have no tasks yet (a group exists through its tasks).
  const [newGroups, setNewGroups] = useState<string[]>([]);
  const [groupDialog, setGroupDialog] = useState<
    { kind: "new"; moving: Task[] | null } | { kind: "rename"; from: string } | null
  >(null);
  const [takeoverOf, setTakeoverOf] = useState<Task | null>(null);
  const [restoreOf, setRestoreOf] = useState<Task[] | null>(null);
  /** Recorded renames, or a task that never backed up here (an older version may have renamed without a record). */
  const cobianRestorable = (t: Task) => (snap.cobianRenamed?.[t.id] ?? 0) > 0 || !snap.states[t.id]?.lastBackup;
  const [manualRun, setManualRun] = useState<{ ids: string[]; mode: BackupMode | null } | null>(null);
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
  const [order, setOrder] = useCustomOrder();
  const custom = sort.key === "custom";
  const checkCol = custom ? CHECK_COL_CUSTOM : CHECK_COL;
  const [compact, setCompact] = useCompactView();
  const cols = useColumnWidths("backuper.taskColumns", DEFAULT_WIDTHS);
  const colWidth = (k: ColumnKey) => (compact ? COMPACT_COLS[k] : undefined) ?? cols.widths[k];
  const nameAuto = cols.widths.name === 0;
  // The table never scrolls sideways: its width is measured and the columns shrink to fit (see fitColumns).
  const [tableWrap, setTableWrap] = useState<HTMLDivElement | null>(null);
  const [availWidth, setAvailWidth] = useState(0);
  useEffect(() => {
    if (!tableWrap) return;
    const ro = new ResizeObserver(() => setAvailWidth(Math.floor(tableWrap.clientWidth)));
    ro.observe(tableWrap);
    return () => ro.disconnect();
  }, [tableWrap]);
  const room = availWidth - checkCol - MIN_ACTIONS_COL;
  // When even the floors don't fit, the least important columns drop out (the task name and status stay readable).
  const floors: Record<ColumnKey, number> = compact ? { ...COL_FLOORS, ...COMPACT_COLS } : COL_FLOORS;
  let shownCols = RESIZABLE;
  if (availWidth > 0) {
    for (const k of NARROW_DROP) {
      if (shownCols.reduce((s, j) => s + floors[j], 0) > room) shownCols = shownCols.filter((j) => j !== k);
    }
  }
  const showCol = (k: ColumnKey) => shownCols.includes(k);
  const nameHeader = useRef<HTMLTableCellElement>(null);
  const fitted = fitColumns(
    Object.fromEntries(
      shownCols.map((k) => [k, k === "name" && nameAuto ? MIN_NAME_COL : Math.max(floors[k], colWidth(k))]),
    ) as Record<ColumnKey, number>,
    floors,
    room,
  );
  /** The width a column really has on screen (the auto name column fills what's left). */
  const shownWidth = (k: ColumnKey) =>
    k === "name" && nameAuto ? (nameHeader.current?.getBoundingClientRect().width ?? fitted.name) : fitted[k];
  // Once a border is dragged, every column gets a fixed width (what's shown now), so only the dragged border moves.
  const freezeColumns = () => {
    const now: Partial<Record<ColumnKey, number>> = {};
    for (const k of shownCols) if (!(compact && COMPACT_COLS[k])) now[k] = Math.floor(shownWidth(k));
    cols.setMany(now);
  };
  /** Dragging may not make the columns wider than the table. */
  const resizeColumn = (k: ColumnKey, w: number) => {
    w = Math.max(w, floors[k]);
    if (availWidth <= 0) return cols.setWidth(k, w);
    const others = shownCols.filter((j) => j !== k).reduce((s, j) => s + shownWidth(j), 0);
    cols.setWidth(k, Math.max(floors[k], Math.min(w, room - others)));
  };
  const transfer = useTaskListTransfer(snap.tasks, refresh);
  // Read again whenever a backup starts or ends (it changes the free space).
  const space = useDiskSpace(
    snap.tasks.map((t) => t.destination).filter(Boolean),
    `${snap.current?.runId ?? ""}|${Object.values(snap.states).map((s) => s.lastRunAt ?? "").join()}`,
  );
  // What each task's next full backup needs; also re-read when a setting that decides it changes.
  const fullSpace = useFullSpace(
    snap.tasks.filter((t) => t.destination).map((t) => t.id),
    `${snap.current?.runId ?? ""}|${snap.tasks
      .map((t) => [t.id, t.destination, t.mode, !!t.fullSchedule, t.keepMode, t.keepCount, t.keepDays, t.deleteBefore, t.reusePrevious, t.deleteEmptyIncrementals, t.sources.length].join(":"))
      .join()}|${Object.values(snap.states).map((s) => s.lastRunAt ?? "").join()}`,
  );

  const usedGroups = groupNames(snap.tasks);
  const allGroups = [...usedGroups, ...newGroups.filter((n) => !usedGroups.some((g) => sameGroup(g, n)))];
  const ungrouped = snap.tasks.filter((t) => !groupOf(t)).length;
  // A tab whose group no longer exists falls back to all tasks.
  const group =
    activeGroup === null
      ? null
      : activeGroup === ""
        ? ungrouped > 0 && allGroups.length > 0
          ? ""
          : null
        : (allGroups.find((g) => sameGroup(g, activeGroup)) ?? null);
  const inGroup = (t: Task) => group === null || sameGroup(groupOf(t), group);
  const groupTasksList = snap.tasks.filter(inGroup);

  const tasks = useMemo(() => {
    const q = query.trim().toLowerCase();
    const shown = snap.tasks.filter(inGroup);
    const found = q
      ? shown.filter((t) => [t.name, t.destination, ...t.sources.map((s) => s.path)].some((s) => s.toLowerCase().includes(q)))
      : shown;
    return sortTasks(found, sort, snap.states, order);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snap.tasks, snap.states, query, sort, group, order]);
  const groups = useMemo(() => groupTasks(tasks, sort.key), [tasks, sort.key]);

  const changeSort = (key: SortKey) => {
    // The first switch to the custom order starts from what is shown now, so it can be fine-tuned from there.
    if (key === "custom" && order.length === 0) setOrder(sortTasks(snap.tasks, sort, snap.states).map((t) => t.id));
    setSort({ ...sort, key });
  };

  // Custom order: a task is dragged by its handle (pointer events, so it works the same in WebView2 and a browser)
  // or moved with the arrow keys on the handle / the row menu.
  const [drag, setDrag] = useState<{ id: string; over: string | null; after: boolean } | null>(null);
  const tbodyRef = useRef<HTMLTableSectionElement>(null);
  const scrollerRef = useRef<HTMLElement | null>(null);
  const focusHandle = (id: string) =>
    requestAnimationFrame(() => tbodyRef.current?.querySelector<HTMLElement>(`[data-handle="${CSS.escape(id)}"]`)?.focus());
  const moveTask = (id: string, target: string, after: boolean) => setOrder((o) => moveInOrder(snap.tasks, o, id, target, after));
  /** One place up (-1) or down (1) among the tasks shown. */
  const moveBy = (id: string, delta: -1 | 1) => {
    const i = tasks.findIndex((t) => t.id === id);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= tasks.length) return;
    moveTask(id, tasks[j].id, delta > 0);
    focusHandle(id);
  };
  /** The row under the pointer and whether the drop goes after it (lower half). */
  const dropAt = (y: number) => {
    const rows = [...(tbodyRef.current?.querySelectorAll<HTMLElement>("tr[data-task-id]") ?? [])];
    for (let i = 0; i < rows.length; i++) {
      const b = rows[i].getBoundingClientRect();
      if (y < b.bottom || i === rows.length - 1) return { over: rows[i].dataset.taskId!, after: y > b.top + b.height / 2 };
    }
    return { over: null, after: false };
  };
  const startDrag = (e: ReactPointerEvent<HTMLElement>, id: string) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    let el: HTMLElement | null = e.currentTarget.parentElement;
    while (el && !/(auto|scroll)/.test(getComputedStyle(el).overflowY)) el = el.parentElement;
    scrollerRef.current = el;
    setDrag({ id, over: null, after: false });
  };
  const dragMove = (e: ReactPointerEvent<HTMLElement>, id: string) => {
    if (drag?.id !== id) return;
    const sc = scrollerRef.current;
    if (sc) {
      const b = sc.getBoundingClientRect();
      if (e.clientY < b.top + 48) sc.scrollBy(0, -16);
      else if (e.clientY > b.bottom - 48) sc.scrollBy(0, 16);
    }
    setDrag({ id, ...dropAt(e.clientY) });
  };
  const endDrag = (id: string) => {
    if (drag?.id === id && drag.over && drag.over !== id) moveTask(id, drag.over, drag.after);
    setDrag(null);
  };

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

  const runNow = async (ids: string[], mode: BackupMode | null = null, fresh = false) => {
    const n = await api.runTasks(ids, mode, fresh);
    if (n === 0) toast({ tone: "info", title: "המשימה כבר רצה או ממתינה בתור" });
    else toast({ tone: "info", title: n === 1 ? "הגיבוי נוסף לתור" : `${n} גיבויים נוספו לתור` });
  };

  const { resume, restart: restartTask, dismiss } = useResumeTask();
  const restart = (task: Task, mode: BackupMode | null) => restartTask(task.id, task.name, mode);

  // Tasks imported from Cobian wait for the decision about Cobian's backups before anything runs.
  // The question comes up when one is switched on, or when someone runs it by hand.
  const run = async (ids: string[], mode: BackupMode | null = null) => {
    const waiting = snap.tasks.filter((t) => ids.includes(t.id) && t.cobianPending).map((t) => t.id);
    const rest = ids.filter((id) => !waiting.includes(id));
    // A paused backup: continue it, or start a new one (a scheduled run always starts a new one).
    const fresh: string[] = [];
    const go: string[] = [];
    for (const id of rest) {
      const task = snap.tasks.find((t) => t.id === id);
      if (!task || !snap.states[id]?.resumable) {
        go.push(id);
        continue;
      }
      const answer = await choose({
        title: `ל"${task.name}" יש גיבוי מושהה`,
        message:
          "אפשר להמשיך אותו מאותה נקודה, או להתחיל גיבוי חדש מההתחלה. הגיבוי החלקי הישן יימחק בהמשך, אחרי שגיבוי חדש יסתיים.",
        options: [
          { value: "continue", label: "המשך מאותה נקודה" },
          { value: "fresh", label: "התחלה מחדש" },
        ],
      });
      if (answer === "continue") go.push(id);
      else if (answer === "fresh") fresh.push(id);
    }
    if (go.length > 0) await runNow(go, mode);
    if (fresh.length > 0) await runNow(fresh, mode, true);
    if (waiting.length > 0) setManualRun({ ids: waiting, mode });
  };
  const askCobian = snap.tasks.filter((t) => t.cobianPending && (t.enabled || manualRun?.ids.includes(t.id)));
  const cobianDecided = () => {
    const m = manualRun;
    setManualRun(null);
    refresh();
    if (m) void runNow(m.ids, m.mode);
  };
  /** "Later": switched-on tasks go back to switched off (they're asked again when switched on). */
  const cobianLater = async () => {
    setManualRun(null);
    const on = askCobian.filter((t) => t.enabled);
    if (on.length === 0) return;
    try {
      await api.saveTasks(on.map((t) => ({ ...t, enabled: false })));
      refresh();
      toast({
        tone: "info",
        title: on.length === 1 ? "המשימה נשארה מושבתת" : `${on.length} משימות נשארו מושבתות`,
        message: "תישאלו אם להמשיך מהגיבויים של Cobian כשתפעילו אותן.",
      });
    } catch (e) {
      toast({ tone: "bad", title: "השמירה נכשלה", message: errorText(e) });
    }
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
      list.length === 1
        ? enabled
          ? "המשימה הופעלה"
          : "המשימה הושבתה"
        : enabled
          ? `${list.length} משימות הופעלו`
          : `${list.length} משימות הושבתו`,
    );

  const remove = async (list: Task[]) => {
    const ok = await confirm({
      title: list.length === 1 ? `למחוק את המשימה "${list[0].name}"?` : `למחוק ${list.length} משימות?`,
      message: "המשימות יוסרו מהתוכנה. קבצי הגיבוי שכבר נוצרו ביעד לא יימחקו.",
      confirmLabel: "מחיקה",
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

  /** Moves tasks into a group ("" = general). */
  const moveToGroup = (list: Task[], to: string) =>
    saveWithUndo(
      list.map((t) => ({ ...t, group: to })),
      list,
      `${list.length === 1 ? "המשימה הועברה" : `${list.length} משימות הועברו`} ל${to ? `קבוצה "${to}"` : `${GENERAL_GROUP}`}`,
    );

  const renameGroup = (from: string, to: string) => {
    const list = snap.tasks.filter((t) => sameGroup(groupOf(t), from));
    setNewGroups((n) => n.map((g) => (sameGroup(g, from) ? to : g)));
    setActiveGroup(to);
    if (list.length > 0)
      saveWithUndo(
        list.map((t) => ({ ...t, group: to })),
        list,
        `שם הקבוצה שונה ל"${to}"`,
      );
  };

  const dissolveGroup = async (name: string) => {
    const list = snap.tasks.filter((t) => sameGroup(groupOf(t), name));
    setNewGroups((n) => n.filter((g) => !sameGroup(g, name)));
    setActiveGroup(null);
    if (list.length === 0) return;
    const ok = await confirm({
      title: `לבטל את הקבוצה "${name}"?`,
      message: `${list.length === 1 ? "המשימה שבה תעבור" : `${list.length} המשימות שבה יעברו`} לרשימה הכללית. המשימות עצמן והגיבויים שלהן לא נמחקים.`,
      confirmLabel: "ביטול הקבוצה",
    });
    if (ok) moveToGroup(list, "");
  };

  /** "Move to group" entries: every other group, the general list, and a new group. `long` = for a menu with other actions. */
  const moveItems = (list: Task[], long = false): MenuItem[] => [
    ...allGroups
      .filter((g) => !list.every((t) => sameGroup(groupOf(t), g)))
      .map((g) => ({
        label: long ? `העברה לקבוצה: ${g}` : g,
        icon: <FolderInput size={14} />,
        onClick: () => moveToGroup(list, g),
      })),
    ...(list.some((t) => groupOf(t))
      ? [
          {
            label: long ? `העברה ל${GENERAL_GROUP} (ללא קבוצה)` : `${GENERAL_GROUP} (ללא קבוצה)`,
            icon: <FolderInput size={14} />,
            onClick: () => moveToGroup(list, ""),
          },
        ]
      : []),
    ...(long ? [] : ["separator" as const]),
    {
      label: long ? "העברה לקבוצה חדשה..." : "קבוצה חדשה...",
      icon: <FolderPlus size={14} />,
      onClick: () => setGroupDialog({ kind: "new", moving: list }),
    },
  ];

  const queuedIds = new Set(snap.queue.map((j) => j.taskId));
  const runningId = snap.current?.taskId;
  const pausedTasks = snap.tasks.filter(
    (t) => snap.states[t.id]?.lastStatus === "cancelled" && snap.states[t.id]?.resumable && t.id !== runningId && !queuedIds.has(t.id),
  );

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
              <Select aria-label="מיון" className="w-56" value={sort.key} onChange={(e) => changeSort(e.target.value as SortKey)}>
                {(Object.keys(SORT_LABEL) as SortKey[]).map((k) => (
                  <option key={k} value={k}>
                    מיון: {SORT_LABEL[k]}
                  </option>
                ))}
              </Select>
              {!custom && (
                <IconButton
                  label={sort.desc ? "סדר יורד - לחצו לסדר עולה" : "סדר עולה - לחצו לסדר יורד"}
                  onClick={() => setSort({ ...sort, desc: !sort.desc })}
                  className="size-9 border border-line bg-panel"
                >
                  {sort.desc ? <ArrowUpZA size={16} /> : <ArrowDownAZ size={16} />}
                </IconButton>
              )}
            </div>
          )}
          {snap.tasks.length > 0 && (
            <IconButton
              label={compact ? "תצוגה רגילה (עם טקסט במצב ובסוג)" : "תצוגה קומפקטית (בלי טקסט במצב ובסוג)"}
              aria-pressed={compact}
              onClick={() => setCompact(!compact)}
              className={cx("size-9 border", compact ? "border-accent/40 bg-accent-soft text-accent" : "border-line bg-panel")}
            >
              <Minimize2 size={16} />
            </IconButton>
          )}
          {snap.tasks.length > 0 && (
            <SearchBox value={query} onChange={setQuery} placeholder="חיפוש..." className="w-44 sm:w-56" />
          )}
          <Menu
            trigger={(open) => (
              <Button icon={<ArrowDownUp size={16} />} onClick={open}>
                ייבוא וייצוא
              </Button>
            )}
            items={[
              { label: "ייבוא מ-Cobian Reflector / Backup", icon: <History size={14} />, onClick: transfer.importCobian },
              { label: "ייבוא משימות מקובץ (Backuper או Cobian)", icon: <Import size={14} />, onClick: transfer.importFile },
              {
                label: "ייצוא כל המשימות לקובץ",
                icon: <Download size={14} />,
                disabled: snap.tasks.length === 0,
                onClick: () => transfer.exportTasks(),
              },
            ]}
          />
          <Button variant="primary" icon={<Plus size={16} />} onClick={() => setEditing({ ...newTask(), group: group ?? "" })}>
            משימה חדשה
          </Button>
        </div>
      </header>

      {snap.tasks.length > 0 && (
        <nav aria-label="קבוצות משימות" className="flex flex-wrap items-center gap-1.5">
          {allGroups.length > 0 && (
            <>
              <GroupTab
                label="כל המשימות"
                count={snap.tasks.length}
                active={group === null}
                onClick={() => setActiveGroup(null)}
              />
              {allGroups.map((g) => (
                <GroupTab
                  key={g}
                  label={g}
                  count={snap.tasks.filter((t) => sameGroup(groupOf(t), g)).length}
                  active={group !== null && sameGroup(group, g)}
                  onClick={() => setActiveGroup(g)}
                />
              ))}
              {ungrouped > 0 && (
                <GroupTab label={GENERAL_GROUP} count={ungrouped} active={group === ""} onClick={() => setActiveGroup("")} />
              )}
            </>
          )}
          <Button
            size="sm"
            variant="ghost"
            icon={<FolderPlus size={15} />}
            onClick={() => setGroupDialog({ kind: "new", moving: null })}
          >
            קבוצה חדשה
          </Button>
          {group && (
            <Menu
              trigger={(open) => (
                <IconButton label={`פעולות על הקבוצה ${group}`} onClick={open} className="ms-auto">
                  <MoreVertical size={16} />
                </IconButton>
              )}
              items={[
                {
                  label: "הרצת כל המשימות הפעילות בקבוצה",
                  icon: <RunIcon size={14} />,
                  disabled: !groupTasksList.some((t) => t.enabled),
                  onClick: () => run(groupTasksList.filter((t) => t.enabled).map((t) => t.id)),
                },
                {
                  label: "ייצוא הקבוצה לקובץ",
                  icon: <Download size={14} />,
                  disabled: groupTasksList.length === 0,
                  onClick: () => transfer.exportTasks(groupTasksList, group),
                },
                "separator",
                {
                  label: "שינוי שם הקבוצה",
                  icon: <Pencil size={14} />,
                  onClick: () => setGroupDialog({ kind: "rename", from: group }),
                },
                { label: "ביטול הקבוצה", icon: <X size={14} />, onClick: () => dissolveGroup(group) },
              ]}
            />
          )}
        </nav>
      )}

      {snap.current && <RunningCard progress={snap.current} queue={snap.queue} tasks={snap.tasks} />}

      <PausedCard
        tasks={pausedTasks}
        states={snap.states}
        onResume={resume}
        onRestart={restart}
        onDetails={showLastRun}
        onDismiss={(t) => dismiss(t.id, t.name)}
      />

      {sel.length > 0 && (
        <div className="sticky top-0 z-10 flex flex-wrap items-center gap-2 rounded-xl border border-accent/30 bg-accent-soft px-3 py-2 shadow-sm">
          <span className="text-sm font-medium text-accent">נבחרו {sel.length}</span>
          <div className="mx-1 h-5 w-px bg-accent/20" />
          {selTasks.some((t) => taskKind(t) === "combined") ? (
            <Menu
              trigger={(open) => (
                <Button size="sm" variant="ghost" icon={<RunIcon size={15} />} onClick={open}>
                  הרצה
                </Button>
              )}
              items={[
                { label: "לפי סוג כל משימה", icon: <RunIcon size={14} />, onClick: () => run(sel) },
                ...runModeItems((mode) => run(sel, mode)),
              ]}
            />
          ) : (
            <Button size="sm" variant="ghost" icon={<RunIcon size={15} />} onClick={() => run(sel)}>
              הרצה
            </Button>
          )}
          <Button size="sm" variant="ghost" icon={<ListChecks size={15} />} onClick={() => setBulk(selTasks)}>
            עריכה מרובה
          </Button>
          <Menu
            trigger={(open) => (
              <Button size="sm" variant="ghost" icon={<FolderInput size={15} />} onClick={open}>
                העברה לקבוצה
              </Button>
            )}
            items={moveItems(selTasks)}
          />
          <Button size="sm" variant="ghost" icon={<Download size={15} />} onClick={() => transfer.exportTasks(selTasks)}>
            ייצוא
          </Button>
          {selTasks.some(cobianRestorable) && (
            <Button size="sm" variant="ghost" icon={<Undo2 size={15} />} onClick={() => setRestoreOf(selTasks.filter(cobianRestorable))}>
              החזרת שמות Cobian
            </Button>
          )}
          <Button size="sm" variant="ghost" icon={<Power size={15} />} onClick={() => setEnabled(selTasks, true)}>
            הפעלה
          </Button>
          <Button size="sm" variant="ghost" icon={<PowerOff size={15} />} onClick={() => setEnabled(selTasks, false)}>
            השבתה
          </Button>
          <Button size="sm" variant="ghost" className="text-bad" icon={<Trash2 size={15} />} onClick={() => remove(selTasks)}>
            מחיקה
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
        ) : groupTasksList.length === 0 ? (
          <EmptyState icon={<FolderPlus size={26} />} title={`אין עדיין משימות בקבוצה "${group}"`}>
            <p>צרו משימה חדשה בקבוצה, או סמנו משימות ברשימה של כל המשימות והעבירו אותן לכאן עם &quot;העברה לקבוצה&quot;.</p>
            <div className="mt-4 flex flex-wrap justify-center gap-2">
              <Button
                variant="primary"
                icon={<Plus size={16} />}
                onClick={() => setEditing({ ...newTask(), group: group ?? "" })}
              >
                משימה חדשה בקבוצה
              </Button>
              <Button onClick={() => setActiveGroup(null)}>לכל המשימות</Button>
            </div>
          </EmptyState>
        ) : tasks.length === 0 ? (
          <div className="p-8 text-center text-sm text-muted">אין משימות שתואמות לחיפוש</div>
        ) : (
          <div ref={setTableWrap} className="overflow-hidden">
            <table className="w-full table-fixed text-sm">
              <colgroup>
                <col style={{ width: checkCol }} />
                {shownCols.map((k) => (
                  <col key={k} style={{ width: k === "name" && nameAuto ? undefined : fitted[k] }} />
                ))}
                <col style={{ width: nameAuto ? MIN_ACTIONS_COL : undefined }} />
              </colgroup>
              <thead>
                <tr className="border-b border-line bg-panel2 text-xs text-muted">
                  <th className="px-3 py-2.5">
                    <Checkbox
                      label="בחירת הכול"
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
                  {shownCols.map((k) => (
                    <th
                      key={k}
                      ref={k === "name" ? nameHeader : undefined}
                      className="relative px-2 py-2.5 text-start font-medium"
                    >
                      {COLUMN_LABEL[k]}
                      {!(compact && COMPACT_COLS[k]) && (
                        <ColumnResizer
                          label={`רוחב העמודה ${COLUMN_LABEL[k]}`}
                          onStart={freezeColumns}
                          onResize={(w) => resizeColumn(k, w)}
                          onReset={() => cols.reset(k)}
                        />
                      )}
                    </th>
                  ))}
                  <th className="px-2 py-2.5" />
                </tr>
              </thead>
              <tbody ref={tbodyRef}>
                {groups.map((g, gi) => (
                  <Fragment key={g.folder ?? gi}>
                    {g.folder !== null && (
                      <tr className="border-b border-line bg-panel2/70">
                        <td colSpan={shownCols.length + 2} className="px-3 py-1.5">
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
                      const dropHere = drag && drag.over === t.id && drag.id !== t.id ? (drag.after ? "after" : "before") : null;
                      return (
                        <tr
                          key={t.id}
                          data-task-id={t.id}
                          onDoubleClick={() => setEditing(t)}
                          className={cx(
                            "group cursor-default border-b border-line last:border-b-0 transition-colors",
                            checked ? "bg-accent-soft/60" : "hover:bg-hover/60",
                            !t.enabled && "text-muted",
                            drag?.id === t.id && "opacity-50",
                            dropHere === "before" && "[&>td]:shadow-[inset_0_2px_0_var(--color-accent)]",
                            dropHere === "after" && "[&>td]:shadow-[inset_0_-2px_0_var(--color-accent)]",
                          )}
                        >
                          <td
                            className={cx("cursor-pointer py-3", custom ? "ps-1 pe-3" : "px-3")}
                            onClick={() => toggle(t.id, !checked)}
                            onDoubleClick={(e) => e.stopPropagation()}
                          >
                            <div className="flex items-center gap-1">
                              {custom && (
                                <button
                                  type="button"
                                  data-handle={t.id}
                                  aria-label={`שינוי המיקום של ${t.name} (חץ למעלה או למטה)`}
                                  title="גררו כדי לשנות את המיקום"
                                  className={cx(
                                    "flex size-6 shrink-0 touch-none items-center justify-center rounded text-muted hover:bg-hover hover:text-fg focus-visible:outline-2 focus-visible:outline-accent",
                                    drag?.id === t.id ? "cursor-grabbing" : "cursor-grab",
                                  )}
                                  onClick={(e) => e.stopPropagation()}
                                  onPointerDown={(e) => startDrag(e, t.id)}
                                  onPointerMove={(e) => dragMove(e, t.id)}
                                  onPointerUp={() => endDrag(t.id)}
                                  onPointerCancel={() => setDrag(null)}
                                  onKeyDown={(e) => {
                                    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
                                      e.preventDefault();
                                      moveBy(t.id, e.key === "ArrowUp" ? -1 : 1);
                                    } else if (e.key === "Escape" && drag) {
                                      e.preventDefault();
                                      setDrag(null);
                                    }
                                  }}
                                >
                                  <GripVertical size={15} />
                                </button>
                              )}
                              <Checkbox label={`בחירת ${t.name}`} checked={checked} onChange={(on) => toggle(t.id, on)} />
                            </div>
                          </td>
                          <td className="px-2 py-3">
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                setEditing(t);
                              }}
                              className="flex max-w-full flex-wrap items-center gap-x-2 gap-y-0.5 text-start font-medium text-fg hover:text-accent"
                            >
                              <span className="max-w-full truncate">{t.name}</span>
                              {!showCol("kind") && (
                                <span className="shrink-0 text-muted" title={KIND_LABEL[taskKind(t)]}>
                                  {KIND_ICON[taskKind(t)]}
                                  <span className="sr-only">{KIND_LABEL[taskKind(t)]}</span>
                                </span>
                              )}
                              {group === null && groupOf(t) && <Badge>{groupOf(t)}</Badge>}
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
                          <td className="px-2 py-3" onClick={(e) => e.stopPropagation()} onDoubleClick={(e) => e.stopPropagation()}>
                            <div className="flex items-center gap-2">
                              <Toggle
                                checked={t.enabled}
                                onChange={(on) => setEnabled([t], on)}
                                ariaLabel={`${t.enabled ? "השבתת" : "הפעלת"} ${t.name}`}
                              />
                              {!compact && (
                                <span className={cx("text-[13px]", t.enabled ? "text-ok" : "text-muted")}>
                                  {t.enabled ? "פעילה" : "מושבתת"}
                                </span>
                              )}
                            </div>
                          </td>
                          {showCol("kind") && (
                            <td className="px-2 py-3">
                              <span title={compact ? KIND_LABEL[taskKind(t)] : undefined}>
                                <Badge tone={taskKind(t) === "incremental" ? "neutral" : "accent"}>
                                  {compact ? (
                                    <>
                                      {KIND_ICON[taskKind(t)]}
                                      <span className="sr-only">{KIND_LABEL[taskKind(t)]}</span>
                                    </>
                                  ) : (
                                    KIND_LABEL[taskKind(t)]
                                  )}
                                </Badge>
                              </span>
                            </td>
                          )}
                          {showCol("schedule") && (
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
                          )}
                          <td
                            className="px-2 py-3 text-[13px]"
                            title={
                              showCol("schedule")
                                ? undefined
                                : t.fullSchedule && t.mode === "incremental"
                                  ? `אינקרמנטלי: ${describeSchedule(t.schedule)}\nמלא: ${describeSchedule(t.fullSchedule)}`
                                  : describeSchedule(t.schedule)
                            }
                          >
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
                                {statusBadge(st.lastStatus, st.resumable)}
                                <span className="text-xs text-muted">{fmtSmart(st.lastRunAt)}</span>
                              </button>
                            ) : (
                              <span className="text-muted">טרם רץ</span>
                            )}
                            <div className="mt-1 flex flex-col text-xs text-muted">
                              {st?.lastBackup && <BackupSize last={st.lastBackup} onOpen={() => setBackupsOf(t)} />}
                              <FreeSpace space={space[t.destination]} need={fullSpace[t.id]} deleteBefore={t.deleteBefore} />
                            </div>
                          </td>
                          <td className="px-2 py-3" onClick={(e) => e.stopPropagation()}>
                            <div className="flex items-center justify-end gap-0.5">
                              {running ? (
                                <>
                                  <IconButton label="השהיה (אפשר להמשיך אחר כך)" onClick={() => api.cancelTask(t.id)}>
                                    <Pause size={15} />
                                  </IconButton>
                                  <IconButton label="עצירה" tone="danger" onClick={() => stopTask(t.id, t.name)}>
                                    <Square size={15} />
                                  </IconButton>
                                </>
                              ) : queued ? (
                                <IconButton label="הסרה מהתור" onClick={() => api.cancelTask(t.id)}>
                                  <X size={15} />
                                </IconButton>
                              ) : st?.lastStatus === "cancelled" && st.resumable ? (
                                <Button
                                  size="sm"
                                  variant="secondary"
                                  className="gap-1 border-warn/40 px-2 text-warn hover:bg-warn-soft"
                                  icon={<RunIcon size={14} />}
                                  title="המשך הגיבוי המושהה מאותה נקודה"
                                  onClick={() => resume(t.id)}
                                >
                                  המשך
                                </Button>
                              ) : taskKind(t) === "combined" ? (
                                <Menu
                                  trigger={(open) => (
                                    <IconButton label="הרצה עכשיו" onClick={open}>
                                      <RunIcon size={16} />
                                    </IconButton>
                                  )}
                                  items={runModeItems((mode) => run([t.id], mode))}
                                />
                              ) : (
                                <IconButton label="הרצה עכשיו" onClick={() => run([t.id])}>
                                  <RunIcon size={16} />
                                </IconButton>
                              )}
                              <Menu
                                trigger={(open) => (
                                  <IconButton label="פעולות נוספות" onClick={open}>
                                    <MoreVertical size={16} />
                                  </IconButton>
                                )}
                                items={[
                                  ...(st?.lastStatus === "cancelled" && st.resumable && !running && !queued
                                    ? ([
                                        {
                                          label: "המשך מאותה נקודה",
                                          icon: <RunIcon size={14} />,
                                          onClick: () => resume(t.id),
                                        },
                                        {
                                          label: "התחלה מחדש",
                                          icon: <RotateCcw size={14} />,
                                          onClick: () => restart(t, null),
                                        },
                                      ] satisfies MenuItem[])
                                    : runModeItems((mode) => run([t.id], mode))),
                                  "separator",
                                  ...(custom
                                    ? ([
                                        {
                                          label: "הזזה למעלה",
                                          icon: <ArrowUp size={14} />,
                                          disabled: tasks[0]?.id === t.id,
                                          onClick: () => moveBy(t.id, -1),
                                        },
                                        {
                                          label: "הזזה למטה",
                                          icon: <ArrowDown size={14} />,
                                          disabled: tasks[tasks.length - 1]?.id === t.id,
                                          onClick: () => moveBy(t.id, 1),
                                        },
                                        "separator",
                                      ] as MenuItem[])
                                    : []),
                                  { label: "עריכה", icon: <Pencil size={14} />, onClick: () => setEditing(t) },
                                  { label: "שכפול", icon: <Copy size={14} />, onClick: () => duplicate(t) },
                                  ...moveItems([t], true),
                                  {
                                    label: t.enabled ? "השבתה" : "הפעלה",
                                    icon: t.enabled ? <PowerOff size={14} /> : <Power size={14} />,
                                    onClick: () => setEnabled([t], !t.enabled),
                                  },
                                  "separator",
                                  { label: "ניהול גיבויים", icon: <FolderTree size={14} />, onClick: () => setBackupsOf(t) },
                                  { label: "המשך מגיבויי Cobian", icon: <History size={14} />, onClick: () => setTakeoverOf(t) },
                                  ...(cobianRestorable(t)
                                    ? ([
                                        {
                                          label: "החזרת השמות של Cobian",
                                          icon: <Undo2 size={14} />,
                                          onClick: () => setRestoreOf([t]),
                                        },
                                      ] satisfies MenuItem[])
                                    : []),
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
      {snap.tasks.length > 0 && (
        <p className="text-xs text-muted">
          טיפ: לחיצה על שורה בוחרת אותה, לחיצה כפולה פותחת עריכה.
          {custom && " בסדר מותאם אישית גוררים משימה בידית שליד תיבת הסימון כדי לשנות את מקומה."}
        </p>
      )}

      {editing && (
        <TaskEditor
          task={editing}
          globalFilters={snap.settings.globalFilters}
          groups={allGroups}
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
      {askCobian.length > 0 && <CobianPendingDialog tasks={askCobian} onCancel={cobianLater} onResolved={cobianDecided} />}
      {takeoverOf && (
        <CobianTakeoverDialog
          task={takeoverOf}
          onClose={() => setTakeoverOf(null)}
          onDone={() => {
            setTakeoverOf(null);
            refresh();
          }}
        />
      )}
      {restoreOf && (
        <CobianRestoreDialog
          tasks={restoreOf}
          onClose={() => setRestoreOf(null)}
          onDone={() => {
            setRestoreOf(null);
            refresh();
          }}
        />
      )}
      {groupDialog && (
        <GroupNameDialog
          title={groupDialog.kind === "rename" ? "שינוי שם הקבוצה" : "קבוצה חדשה"}
          initial={groupDialog.kind === "rename" ? groupDialog.from : ""}
          confirmLabel={groupDialog.kind === "rename" ? "שינוי שם" : groupDialog.moving ? "יצירה והעברה" : "יצירה"}
          taken={allGroups}
          onClose={() => setGroupDialog(null)}
          onSubmit={(name) => {
            const d = groupDialog;
            setGroupDialog(null);
            if (d.kind === "rename") return renameGroup(d.from, name);
            if (d.moving) {
              moveToGroup(d.moving, name);
              setSelected(new Set());
            } else setNewGroups((n) => [...n, name]);
            setActiveGroup(name);
          }}
        />
      )}
      {bulk && (
        <BulkEditDialog
          tasks={bulk}
          groups={allGroups}
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

/**
 * Size of the newest backup alone (full or incremental; it may be older than the last run, if that
 * one failed). Opens the task's backups on disk, each with its own size.
 */
function BackupSize({ last, onOpen }: { last: LastBackup; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        onOpen();
      }}
      onDoubleClick={(e) => e.stopPropagation()}
      className="w-fit rounded text-start hover:text-accent hover:underline"
      title={`גודל הגיבוי האחרון שנשמר (${MODE_LABEL[last.mode]}) - ${fmtSmart(last.at)}\nלחיצה להצגת כל הגיבויים שבדיסק ואת הגודל של כל אחד`}
    >
      גודל <bdi dir="ltr" className="whitespace-nowrap">{fmtBytes(last.bytes)}</bdi> ({MODE_LABEL[last.mode]})
    </button>
  );
}

/**
 * Free space on the destination drive; warns when the next full backup (about the size of the
 * previous one) wouldn't fit, counting what delete-before frees first.
 */
function FreeSpace({
  space,
  need,
  deleteBefore,
}: {
  space: DiskSpace | null | undefined;
  need: FullSpace | null | undefined;
  deleteBefore: boolean;
}) {
  if (space === undefined) return null;
  if (space === null) return <span title="כונן היעד לא מחובר כרגע, או שאי אפשר לקרוא אותו">היעד לא מחובר</span>;
  const low = !!need && space.free + need.freed < need.needed;
  return (
    <span
      className={cx("flex items-center gap-1", low && "text-warn")}
      title={
        low
          ? `הגיבוי המלא הבא צפוי לשקול כגודל הקודם (${fmtBytes(need.needed)}), ובכונן היעד פנוי ${fmtBytes(space.free)}` +
            (deleteBefore && need.freed > 0 ? ` (וגם אחרי מחיקת הגיבויים הישנים, ${fmtBytes(need.freed)}, זה לא מספיק)` : "") +
            " - ייתכן שהוא לא ייכנס. כדאי לפנות מקום או לשמור פחות גיבויים"
          : `מקום פנוי בכונן היעד, מתוך ${fmtBytes(space.total)}`
      }
    >
      {low && <AlertTriangle size={12} />}
      <span>
        פנוי <bdi dir="ltr" className="whitespace-nowrap">{fmtBytes(space.free)}</bdi>
      </span>
    </span>
  );
}

function GroupTab({ label, count, active, onClick }: { label: string; count: number; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cx(
        "flex h-8 max-w-56 items-center gap-1.5 rounded-full border px-3 text-[13px] transition-colors",
        active ? "border-accent/40 bg-accent-soft font-medium text-accent" : "border-line bg-panel text-fg hover:bg-hover",
      )}
    >
      <bdi className="truncate">{label}</bdi>
      <span className={cx("text-xs", active ? "text-accent/80" : "text-muted")}>{count}</span>
    </button>
  );
}
