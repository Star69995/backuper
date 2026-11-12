import { useEffect } from "react";
import { api } from "../api";

/**
 * What the window has open with unsaved changes (a task being edited, the global rules...).
 * The backend gets the list (`set_unsaved`): an update it installs on its own (not "update now")
 * closes the app, so it waits until this is empty.
 */
const open = new Map<number, string>();
let nextId = 0;

function sync() {
  api.setUnsaved([...new Set(open.values())]).catch(() => {});
}

/** Call once at startup: a reloaded page starts with nothing unsaved. */
export function resetUnsaved() {
  open.clear();
  sync();
}

/** Something is open with unsaved changes right now. */
export const hasUnsaved = () => open.size > 0;

/** Registers `what` (a short Hebrew description) while it is not null. */
export function useUnsaved(what: string | null) {
  useEffect(() => {
    if (!what) return;
    const id = nextId++;
    open.set(id, what);
    sync();
    return () => {
      open.delete(id);
      sync();
    };
  }, [what]);
}
