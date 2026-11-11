import { useEffect, useState } from "react";
import type { Task, TaskState } from "../types";
import { nextRunOf } from "./format";

export type SortKey = "custom" | "order" | "name" | "nextRun" | "source" | "destination";
export interface SortState {
  key: SortKey;
  desc: boolean;
}

export const SORT_LABEL: Record<SortKey, string> = {
  order: "סדר יצירה",
  custom: "סדר מותאם אישית",
  name: "שם",
  nextRun: "הריצה הבאה",
  source: "תיקיית מקור",
  destination: "תיקיית יעד",
};

const collator = new Intl.Collator("he", { numeric: true, sensitivity: "base" });
/** Paths compare case-insensitively and ignore a trailing backslash. */
const pathKey = (p: string) => p.trim().replace(/\\+$/, "").toLowerCase();

/** The folder a task is sorted/grouped by. With several sources, the first one. */
const folderOf = (t: Task, key: "source" | "destination") => (key === "destination" ? t.destination : (t.sources[0]?.path ?? ""));

/** Next scheduled run as a timestamp; null = none (manual or disabled). */
const nextAt = (t: Task, states: Record<string, TaskState>) => {
  const n = nextRunOf(t, states[t.id]);
  return n ? new Date(n.at).getTime() : null;
};

/**
 * Tasks in the user's own order (ids, remembered per machine). Tasks missing from it (new ones)
 * follow in creation order.
 */
export function applyOrder(tasks: Task[], order: string[]): Task[] {
  const at = new Map(order.map((id, i) => [id, i]));
  const end = order.length;
  return tasks
    .map((t, i) => ({ t, k: at.get(t.id) ?? end + i }))
    .sort((a, b) => a.k - b.k)
    .map((x) => x.t);
}

/** The custom order after moving one task before / after another (positions within the whole list). */
export function moveInOrder(all: Task[], order: string[], id: string, target: string, after: boolean): string[] {
  const ids = applyOrder(all, order)
    .map((t) => t.id)
    .filter((x) => x !== id);
  const i = ids.indexOf(target);
  if (i < 0) return order;
  ids.splice(after ? i + 1 : i, 0, id);
  return ids;
}

export function sortTasks(
  tasks: Task[],
  { key, desc }: SortState,
  states: Record<string, TaskState>,
  order: string[] = [],
): Task[] {
  const dir = desc ? -1 : 1;
  // The custom order has no direction: it is exactly what the user arranged.
  if (key === "custom") return applyOrder(tasks, order);
  if (key === "order") return desc ? [...tasks].reverse() : tasks;
  if (key === "nextRun") {
    const at = new Map(tasks.map((t) => [t.id, nextAt(t, states)]));
    // Tasks without a next run always go last, in either direction.
    return [...tasks].sort((a, b) => {
      const x = at.get(a.id)!;
      const y = at.get(b.id)!;
      if (x === null || y === null) return (x === null ? 1 : 0) - (y === null ? 1 : 0) || collator.compare(a.name, b.name);
      return dir * (x - y || collator.compare(a.name, b.name));
    });
  }
  return [...tasks].sort((a, b) => {
    const primary =
      key === "name" ? collator.compare(a.name, b.name) : collator.compare(pathKey(folderOf(a, key)), pathKey(folderOf(b, key)));
    return dir * (primary || collator.compare(a.name, b.name));
  });
}

export interface TaskGroup {
  /** null = no grouping (single flat list). */
  folder: string | null;
  tasks: Task[];
}

/** When sorted by a folder and some tasks share it, group them under that folder. */
export function groupTasks(sorted: Task[], key: SortKey): TaskGroup[] {
  if (key !== "source" && key !== "destination") return [{ folder: null, tasks: sorted }];
  const groups: TaskGroup[] = [];
  for (const t of sorted) {
    const last = groups[groups.length - 1];
    const folder = folderOf(t, key);
    if (last && pathKey(last.folder!) === pathKey(folder)) last.tasks.push(t);
    else groups.push({ folder, tasks: [t] });
  }
  return groups.length < sorted.length ? groups : [{ folder: null, tasks: sorted }];
}

const STORAGE_KEY = "backuper.taskSort";

/** Sort choice, remembered per machine (falls back silently if storage is unavailable). */
export function useTaskSort() {
  const [sort, setSort] = useState<SortState>(() => {
    try {
      const v = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
      if (v && v.key in SORT_LABEL) return { key: v.key, desc: !!v.desc };
    } catch {
      /* ignore */
    }
    return { key: "order", desc: false };
  });
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(sort));
    } catch {
      /* ignore */
    }
  }, [sort]);
  return [sort, setSort] as const;
}

const ORDER_KEY = "backuper.taskOrder";

/** The custom task order (task ids), remembered per machine. */
export function useCustomOrder() {
  const [order, setOrder] = useState<string[]>(() => {
    try {
      const v = JSON.parse(localStorage.getItem(ORDER_KEY) ?? "null");
      if (Array.isArray(v)) return v.filter((x): x is string => typeof x === "string");
    } catch {
      /* ignore */
    }
    return [];
  });
  useEffect(() => {
    try {
      localStorage.setItem(ORDER_KEY, JSON.stringify(order));
    } catch {
      /* ignore */
    }
  }, [order]);
  return [order, setOrder] as const;
}
