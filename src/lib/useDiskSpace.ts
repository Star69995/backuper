import { useEffect, useState } from "react";
import { api } from "../api";
import type { DiskSpace } from "../types";

/**
 * Free space on the drives of `paths` (null = not connected, undefined = not read yet).
 * Read again when `key` changes (a backup finished), every minute and when the window gets focus.
 */
export function useDiskSpace(paths: string[], key: string): Record<string, DiskSpace | null> {
  const [space, setSpace] = useState<Record<string, DiskSpace | null>>({});
  const list = [...new Set(paths)].sort().join("\n");
  useEffect(() => {
    if (!list) return;
    const ps = list.split("\n");
    let alive = true;
    const read = () =>
      api
        .diskSpace(ps)
        .then((r) => alive && setSpace(Object.fromEntries(ps.map((p, i) => [p, r[i] ?? null]))))
        .catch(() => {});
    read();
    const timer = setInterval(read, 60_000);
    window.addEventListener("focus", read);
    return () => {
      alive = false;
      clearInterval(timer);
      window.removeEventListener("focus", read);
    };
  }, [list, key]);
  return space;
}
