import { useEffect, useState } from "react";

export const MIN_COLUMN_WIDTH = 64;

/**
 * Column widths in px, remembered per machine (falls back silently if storage is unavailable).
 * Unknown/invalid stored values fall back to the defaults.
 */
export function useColumnWidths<K extends string>(storageKey: string, defaults: Record<K, number>) {
  const [widths, setWidths] = useState<Record<K, number>>(() => {
    try {
      const v = JSON.parse(localStorage.getItem(storageKey) ?? "null");
      if (v && typeof v === "object") {
        const out = { ...defaults };
        for (const k of Object.keys(defaults) as K[]) {
          // 0 = auto width (allowed only where it's the default).
          if (typeof v[k] === "number" && (v[k] >= MIN_COLUMN_WIDTH || (v[k] === 0 && defaults[k] === 0))) {
            out[k] = Math.round(v[k]);
          }
        }
        return out;
      }
    } catch {
      /* ignore */
    }
    return defaults;
  });
  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(widths));
    } catch {
      /* ignore */
    }
  }, [storageKey, widths]);

  const setWidth = (k: K, w: number) => setWidths((s) => ({ ...s, [k]: Math.max(MIN_COLUMN_WIDTH, Math.round(w)) }));
  const setMany = (m: Partial<Record<K, number>>) => setWidths((s) => ({ ...s, ...m }));
  const reset = (k: K) => setWidths((s) => ({ ...s, [k]: defaults[k] }));
  return { widths, setWidth, setMany, reset };
}

/**
 * Shrinks column widths so their sum fits `room` (px), so a table never needs sideways scrolling.
 * Each column goes from its wanted width towards its floor by the same fraction (0 = every column at its floor).
 * Without a measured `room` (0) the wanted widths are returned as they are.
 */
export function fitColumns<K extends string>(wanted: Record<K, number>, floors: Record<K, number>, room: number): Record<K, number> {
  const keys = Object.keys(wanted) as K[];
  const total = keys.reduce((s, k) => s + wanted[k], 0);
  if (room <= 0 || total <= room) return wanted;
  const floorTotal = keys.reduce((s, k) => s + floors[k], 0);
  const slack = total - floorTotal;
  const f = slack > 0 ? Math.max(0, Math.min(1, (room - floorTotal) / slack)) : 0;
  const out = { ...wanted };
  for (const k of keys) out[k] = Math.floor(floors[k] + Math.max(0, wanted[k] - floors[k]) * f);
  return out;
}
