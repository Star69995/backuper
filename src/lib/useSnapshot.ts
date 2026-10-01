import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { Snapshot } from "../types";

/** Live view of the core: refetched on "snapshot-changed", progress patched in from "progress". */
export function useSnapshot() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const refresh = useCallback(() => api.snapshot().then(setSnap), []);

  useEffect(() => {
    refresh();
    const unsubs = [
      api.onSnapshotChanged(refresh),
      api.onProgress((p) => setSnap((s) => (s && s.current?.runId === p.runId ? { ...s, current: p } : s))),
    ];
    // Keeps relative times ("in 5 minutes") fresh.
    const timer = setInterval(refresh, 30_000);
    return () => {
      clearInterval(timer);
      unsubs.forEach((u) => u.then((f) => f()));
    };
  }, [refresh]);

  return { snap, refresh };
}

export function useTheme(theme: "system" | "light" | "dark" | undefined) {
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || ((theme ?? "system") === "system" && mq.matches);
      document.documentElement.classList.toggle("dark", dark);
    };
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, [theme]);
}
