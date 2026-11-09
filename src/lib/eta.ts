import { useRef } from "react";
import type { Progress } from "../types";

/** How far back the current speed looks. */
const WINDOW_MS = 60_000;
/** The speed of the last real copy fades out over this long, as this run's own speed settles. */
const BLEND_MS = 60_000;
/** Below this copying time there is no speed of its own yet. */
const MIN_MS = 3_000;
/** The window is only used once it spans this long, else the average since copying began. */
const MIN_WINDOW_MS = 10_000;

interface Sample {
  t: number;
  bytes: number;
}

/** Bytes per second: the recent window, else the whole copy so far, blended with the last copy's speed at first. */
export function copyRate(samples: Sample[], now: number, p: Progress): number | null {
  const started = p.copyStartedAt ? new Date(p.copyStartedAt).getTime() : null;
  const elapsed = started === null ? 0 : now - started;
  let measured: number | null = null;
  if (elapsed >= MIN_MS && p.bytesDone > 0) {
    const overall = p.bytesDone / (elapsed / 1000);
    const first = samples[0];
    const windowed =
      first && now - first.t >= MIN_WINDOW_MS ? (p.bytesDone - first.bytes) / ((now - first.t) / 1000) : 0;
    // The bytes of a file count once it is done, so a long file leaves the window flat: use the average then.
    measured = windowed > 0 ? windowed : overall;
  }
  const expected = p.expectedRate ?? null;
  if (measured === null) return expected;
  if (expected === null) return measured;
  const w = Math.min(1, elapsed / BLEND_MS);
  return w * measured + (1 - w) * expected;
}

/** Seconds until the copy is done, or null when it can't be told yet (no totals, no speed). */
export function useEta(p: Progress, now = Date.now()): number | null {
  const ref = useRef<{ key: string; samples: Sample[] }>({ key: "", samples: [] });
  const key = `${p.runId}/${p.sourceIndex}`;
  if (ref.current.key !== key) ref.current = { key, samples: [] };
  const { samples } = ref.current;
  samples.push({ t: now, bytes: p.bytesDone });
  while (samples.length > 1 && now - samples[1].t >= WINDOW_MS) samples.shift();

  if (p.phase !== "copying" || p.bytesTotal <= 0 || p.totals === "unknown") return null;
  const rate = copyRate(samples, now, p);
  if (!rate || rate <= 0) return null;
  return Math.max(0, (p.bytesTotal - p.bytesDone) / rate);
}
