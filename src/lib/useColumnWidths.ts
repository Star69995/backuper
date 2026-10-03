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
  const reset = (k: K) => setWidths((s) => ({ ...s, [k]: defaults[k] }));
  return { widths, setWidth, reset };
}
