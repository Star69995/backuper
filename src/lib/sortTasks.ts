import { useEffect, useState } from "react";
import type { Task } from "../types";

export type SortKey = "order" | "name" | "source" | "destination";
export interface SortState {
  key: SortKey;
  desc: boolean;
}

export const SORT_LABEL: Record<SortKey, string> = {
  order: "סדר יצירה",
  name: "שם",
  source: "תיקיית מקור",
  destination: "תיקיית יעד",
};

const collator = new Intl.Collator("he", { numeric: true, sensitivity: "base" });
/** Paths compare case-insensitively and ignore a trailing backslash. */
const pathKey = (p: string) => p.trim().replace(/\\+$/, "").toLowerCase();

export function sortTasks(tasks: Task[], { key, desc }: SortState): Task[] {
  const dir = desc ? -1 : 1;
  if (key === "order") return desc ? [...tasks].reverse() : tasks;
  return [...tasks].sort((a, b) => {
    const primary = key === "name" ? collator.compare(a.name, b.name) : collator.compare(pathKey(a[key]), pathKey(b[key]));
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
    if (last && pathKey(last.folder!) === pathKey(t[key])) last.tasks.push(t);
    else groups.push({ folder: t[key], tasks: [t] });
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
