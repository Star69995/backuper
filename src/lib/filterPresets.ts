import type { FilterRule } from "../types";
import { CACHE_FOLDERS } from "./cacheFolders";

/** The rules behind the quick-add buttons of `FilterRulesEditor`; `defaultGroup` is all of them in one row. */
export const TEMP_FILES: FilterRule[] = [
  { kind: "extension", value: "tmp, temp, bak, part, crdownload" },
  { kind: "pattern", value: "~$*" },
];

export const WINDOWS_JUNK: FilterRule[] = [
  { kind: "pattern", value: "Thumbs.db" },
  { kind: "pattern", value: "desktop.ini" },
];

export const CACHE_RULES: FilterRule[] = CACHE_FOLDERS.map((value) => ({ kind: "folder", value }));

/** Files nobody would open from a backup (hidden and system files, temp files, Windows leftovers, cache folders) as one row. */
export const defaultGroup = (): FilterRule => ({
  kind: "group",
  name: "קבצי מערכת ומיותרים",
  enabled: true,
  rules: ([{ kind: "hidden" }, { kind: "system" }, ...TEMP_FILES, ...WINDOWS_JUNK, ...CACHE_RULES] as FilterRule[]).map((r) => ({ ...r })),
});

/** What a new task starts with. */
export const defaultRules = (): FilterRule[] => [defaultGroup()];
