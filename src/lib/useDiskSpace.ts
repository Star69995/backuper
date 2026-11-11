import { useEffect, useState } from "react";
import { api } from "../api";
import type { DiskSpace, FullSpace } from "../types";

/**
 * Reads `read(items)` for the `\n`-joined `list`, again when `key` changes (a backup finished),
 * every minute and when the window gets focus. Items not read yet are missing from the result.
 */
function usePolled<T>(
  list: string,
  key: string,
  read: (items: string[]) => Promise<T[]>,
): Record<string, T> {
  const [data, setData] = useState<Record<string, T>>({});
  useEffect(() => {
    if (!list) return;
    const items = list.split("\n");
    let alive = true;
    const load = () =>
      read(items)
        .then((r) => alive && setData(Object.fromEntries(items.map((p, i) => [p, r[i]]))))
        .catch(() => {});
    load();
    const timer = setInterval(load, 60_000);
    window.addEventListener("focus", load);
    return () => {
      alive = false;
      clearInterval(timer);
      window.removeEventListener("focus", load);
    };
    // `read` only wraps a stable api call.
  }, [list, key]);
  return data;
}

/** Free space on the drives of `paths` (null = not connected, undefined = not read yet). */
export function useDiskSpace(paths: string[], key: string): Record<string, DiskSpace | null> {
  const list = [...new Set(paths)].sort().join("\n");
  return usePolled(list, key, async (ps) => (await api.diskSpace(ps)).map((d) => d ?? null));
}

/**
 * What each task's next full backup needs on its destination (null = no full backup expected,
 * undefined = not read yet). `key` must change with anything that decides it: a backup finishing
 * and the task settings it depends on (type, retention, delete-before, reuse).
 */
export function useFullSpace(taskIds: string[], key: string): Record<string, FullSpace | null> {
  const list = [...new Set(taskIds)].sort().join("\n");
  return usePolled(list, key, async (ids) => (await api.fullSpace(ids)).map((d) => d ?? null));
}
