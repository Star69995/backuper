// Reading a list of filter rules as a whole: tidying it (merge, sort, drop repeats) and finding rules that
// repeat, include or overlap each other. Pure functions, no UI. The matching is a close approximation of the
// engine's (wildcards `*` `?` on names and paths, regex tried on literal names), meant to warn, not to decide.

import type { FilterRule } from "../types";

/** One thing a rule matches, normalized so rules written differently compare equal (`tmp` and `*.tmp`). */
interface Atom {
  /** name: file-name wildcard (skip) - onlyName: file name to keep - dir: folder name or path - path: one file
   * - fileRe / onlyRe / dirRe: regex on file names (skip) / (keep only) / folder names. */
  t: "name" | "onlyName" | "dir" | "path" | "fileRe" | "onlyRe" | "dirRe" | "size" | "age" | "hidden" | "system";
  v: string;
  n?: number;
}

export const splitList = (v: string) =>
  v
    .split(/[,;]/)
    .map((x) => x.trim())
    .filter(Boolean);

/** Same reading as the backend's `patterns`: a bare word or ".ext" is an extension. */
const pattern = (p: string) =>
  /[*?]/.test(p) ? p : p.startsWith(".") ? `*${p}` : !p.includes(".") ? `*.${p}` : p;

const slashes = (v: string) => v.trim().replace(/\//g, "\\");
const lower = (v: string) => v.toLowerCase();

function atomsOf(r: FilterRule): Atom[] {
  switch (r.kind) {
    case "include":
      return splitList(r.value).map((p) => ({ t: "onlyName", v: lower(pattern(p)) }));
    case "includeName":
      return splitList(r.value).map((p) => ({ t: "onlyName", v: lower(p) }));
    case "extension":
      return splitList(r.value).map((p) => ({ t: "name", v: lower(pattern(p)) }));
    case "pattern":
      return splitList(r.value).map((p) => ({ t: "name", v: lower(p) }));
    case "regex":
      return r.value.trim() ? [{ t: r.folders ? "dirRe" : r.include ? "onlyRe" : "fileRe", v: r.value.trim() }] : [];
    case "folder": {
      const v = slashes(r.value).replace(/\\+$/, "");
      return v ? [{ t: "dir", v: lower(v) }] : [];
    }
    case "file": {
      const v = slashes(r.value);
      return v ? [{ t: "path", v: lower(v) }] : [];
    }
    case "largerThan":
      return [{ t: "size", v: "", n: r.mb }];
    case "olderThan":
      return [{ t: "age", v: "", n: r.days }];
    case "hidden":
      return [{ t: "hidden", v: "" }];
    case "system":
      return [{ t: "system", v: "" }];
  }
}

const hasWild = (v: string) => /[*?]/.test(v);

const wildRe = (p: string, underToo = false) =>
  new RegExp(
    `^${p
      .replace(/[.+^${}()|[\]\\]/g, "\\$&")
      .replace(/\*/g, ".*")
      .replace(/\?/g, ".")}${underToo ? "(\\\\.*)?" : ""}$`,
    "i",
  );

const reTest = (re: string, s: string) => {
  try {
    return new RegExp(re, "i").test(s);
  } catch {
    return false;
  }
};

const segments = (path: string) => path.split("\\").filter(Boolean);

/** `a` (a name pattern) matches the name `b`, which may itself hold wildcards. */
function nameCovers(a: string, b: string) {
  if (a === b || a === "*" || a === "*.*") return true;
  if (!hasWild(a)) return false;
  // A wildcard name `b` is inside `a` only when `a` has just `*`s (a `?` could be narrower than b's `*`).
  if (hasWild(b) && a.includes("?")) return false;
  return wildRe(a).test(b);
}

/** Everything `b` matches is matched by `a` too. */
function atomCovers(a: Atom, b: Atom): boolean {
  if (a.t === "size" || a.t === "age") return a.t === b.t && a.n! <= b.n!;
  if (a.t === "hidden" || a.t === "system") return a.t === b.t;
  switch (a.t) {
    case "name":
    case "onlyName":
      if (b.t === a.t) return nameCovers(a.v, b.v);
      // A name pattern also covers a file given by its path (by its last name).
      return a.t === "name" && b.t === "path" && wildRe(a.v).test(segments(b.v).pop() ?? "");
    case "fileRe":
      if (b.t === "fileRe") return a.v === b.v;
      if (b.t === "name" && !hasWild(b.v)) return reTest(a.v, b.v);
      return b.t === "path" && reTest(a.v, segments(b.v).pop() ?? "");
    case "onlyRe":
      return b.t === "onlyRe" && a.v === b.v;
    case "dirRe":
      if (b.t === "dirRe") return a.v === b.v;
      if (b.t === "dir") return segments(b.v).some((s) => !hasWild(s) && reTest(a.v, s));
      return b.t === "path" && segments(b.v).slice(0, -1).some((s) => reTest(a.v, s));
    case "dir": {
      const under = a.v.includes("\\");
      if (b.t === "dir") {
        if (a.v === b.v) return true;
        if (!under) return segments(b.v).some((s) => nameCovers(a.v, s));
        if (!b.v.includes("\\")) return false;
        // A relative path is read under the source, so it also covers the same path deeper in a full one.
        return wildRe(a.v, true).test(b.v) || (!hasWild(a.v) && !/^([a-z]:|\\\\)/i.test(a.v) && `\\${b.v}\\`.includes(`\\${a.v}\\`));
      }
      if (b.t === "path") {
        const up = segments(b.v).slice(0, -1);
        if (!under) return up.some((s) => nameCovers(a.v, s));
        return wildRe(a.v, true).test(up.join("\\")) || (!/^([a-z]:|\\\\)/i.test(a.v) && `\\${up.join("\\")}\\`.includes(`\\${a.v}\\`));
      }
      return false;
    }
    case "path":
      return b.t === "path" && a.v === b.v;
  }
  return false;
}

const atomKey = (a: Atom) => `${a.t}:${a.v}:${a.n ?? ""}`;
const sameAtoms = (x: Atom[], y: Atom[]) => {
  const kx = new Set(x.map(atomKey));
  const ky = new Set(y.map(atomKey));
  return kx.size === ky.size && [...kx].every((k) => ky.has(k));
};

/** How many of `b`'s atoms `a` covers: all = `b` is inside `a`. */
const coveredCount = (a: Atom[], b: Atom[]) => b.filter((y) => a.some((x) => atomCovers(x, y))).length;

export type RelationKind =
  /** Does exactly what the other rule does. */
  | "same"
  /** Everything it matches the other rule matches too. */
  | "inside"
  /** It matches everything the other rule does (that one is redundant). */
  | "contains"
  /** Some of its entries are already matched by the other rule. */
  | "partial";

export interface Relation {
  kind: RelationKind;
  other: FilterRule;
  /** The other rule is one of the global rules. */
  global: boolean;
}

/** How atoms `b` (a rule) relate to atoms `a` (another rule): "inside" = b is inside a, "contains" = b includes a. */
function relate(a: Atom[], b: Atom[]): RelationKind | null {
  if (!a.length || !b.length) return null;
  if (sameAtoms(a, b)) return "same";
  const ab = coveredCount(a, b);
  const ba = coveredCount(b, a);
  if (ab === b.length && ba === a.length) return "same";
  if (ab === b.length) return "inside";
  if (ba === a.length) return "contains";
  return ab > 0 ? "partial" : null;
}

/**
 * For each rule, how it relates to the other rules (and to the global ones). Of two identical rules only
 * the later one is marked, so removing what is marked leaves one of them.
 */
export function relations(rules: FilterRule[], globalRules: FilterRule[] = []): Relation[][] {
  const mine = rules.map(atomsOf);
  const theirs = globalRules.map(atomsOf);
  return mine.map((a, i) => {
    const out: Relation[] = [];
    const add = (k: RelationKind | null, other: FilterRule, global: boolean) => k && out.push({ kind: k, other, global });
    // `a` against `b`: "inside" means a is inside b, so read it from b's side.
    mine.forEach((b, j) => {
      if (i === j) return;
      const k = relate(b, a);
      if (k === "same" && j > i) return;
      add(k, rules[j], false);
    });
    theirs.forEach((b, j) => add(relate(b, a), globalRules[j], true));
    return out;
  });
}

const KIND_ORDER = (r: FilterRule) => {
  switch (r.kind) {
    case "extension":
      return 0;
    case "pattern":
      return 1;
    case "regex":
      return r.folders ? 4 : r.include ? 3 : 2;
    case "folder":
      return 5;
    case "file":
      return 6;
    case "largerThan":
      return 7;
    case "olderThan":
      return 8;
    case "hidden":
      return 9;
    case "system":
      return 10;
    case "include":
      return 11;
    case "includeName":
      return 12;
  }
};

/** A rule with its text cleaned up: trimmed, slashes the Windows way, list entries without repeats, sorted. */
export function normalizeRule(r: FilterRule): FilterRule {
  switch (r.kind) {
    case "include":
    case "includeName":
    case "extension":
    case "pattern": {
      // Entries that mean the same (`tmp`, `.tmp`, `*.TMP` in a file-type rule) count once.
      const key = (x: string) => lower(r.kind === "include" || r.kind === "extension" ? pattern(x) : x);
      const seen = new Set<string>();
      const items = splitList(r.value).filter((x) => !seen.has(key(x)) && seen.add(key(x)));
      items.sort((x, y) => x.localeCompare(y, "en", { sensitivity: "base" }));
      return { ...r, value: items.join(", ") };
    }
    case "folder":
      return { ...r, value: slashes(r.value).replace(/\+$/, "") };
    case "file":
      return { ...r, value: slashes(r.value) };
    case "regex":
      return { ...r, value: r.value.trim() };
    default:
      return r;
  }
}

const isList = (r: FilterRule): r is Extract<FilterRule, { kind: "include" | "includeName" | "extension" | "pattern" }> =>
  r.kind === "include" || r.kind === "includeName" || r.kind === "extension" || r.kind === "pattern";

export interface Tidied {
  rules: FilterRule[];
  /** Rules that were an exact repeat of another and are gone. */
  repeated: number;
  /** Rules of the same file-type/name kind folded into another one. */
  merged: number;
  /** The order or the text changed (even if no rule was removed). */
  changed: boolean;
}

const sortValue = (r: FilterRule) =>
  r.kind === "largerThan" ? String(r.mb).padStart(9, "0") : r.kind === "olderThan" ? String(r.days).padStart(9, "0") : (atomsOf(r)[0]?.v ?? "");

/**
 * Cleans a list: each rule's text, rules of the same file-name kind merged into one (they add up anyway),
 * exact repeats dropped, and the rest ordered by kind, then by value.
 */
export function tidy(rules: FilterRule[]): Tidied {
  let merged = 0;
  let repeated = 0;
  const out: FilterRule[] = [];
  for (const r of rules.map(normalizeRule)) {
    const at = atomsOf(r);
    if (isList(r) && at.length) {
      const i = out.findIndex((x) => x.kind === r.kind && atomsOf(x).length > 0);
      if (i >= 0) {
        out[i] = normalizeRule({ ...out[i], value: `${(out[i] as typeof r).value}, ${r.value}` } as FilterRule);
        merged++;
        continue;
      }
    } else if (out.some((x) => x.kind === r.kind && sameAtoms(atomsOf(x), at))) {
      repeated++;
      continue;
    }
    out.push(r);
  }
  out.sort((a, b) => KIND_ORDER(a) - KIND_ORDER(b) || sortValue(a).localeCompare(sortValue(b), "en", { numeric: true }));
  return { rules: out, repeated, merged, changed: JSON.stringify(out) !== JSON.stringify(rules) };
}

export interface Added {
  rules: FilterRule[];
  added: FilterRule[];
  /** Left out: the rule already exists. */
  same: FilterRule[];
  /** Left out: an existing rule already includes it (with that rule). */
  inside: { rule: FilterRule; by: FilterRule }[];
  /** Added, but some entries were already covered (with the rule that covers them). */
  partial: { rule: FilterRule; by: FilterRule }[];
}

/** Adds rules one by one to a list, leaving out those that are already there or already included in a rule. */
export function addRules(existing: FilterRule[], incoming: FilterRule[]): Added {
  const out: Added = { rules: [...existing], added: [], same: [], inside: [], partial: [] };
  const atoms = existing.map(atomsOf);
  for (const r of incoming) {
    const norm = normalizeRule(r);
    const at = atomsOf(norm);
    const push = () => {
      out.rules.push(norm);
      atoms.push(at);
      out.added.push(norm);
    };
    // A rule with nothing typed yet is only repeated by another empty rule of its kind.
    if (!at.length) {
      if (out.rules.some((x) => JSON.stringify(x) === JSON.stringify(norm))) out.same.push(norm);
      else push();
      continue;
    }
    const found = atoms.map((b, j) => ({ k: relate(b, at), j })).filter((f) => f.k);
    if (found.some((f) => f.k === "same")) out.same.push(norm);
    else if (found.some((f) => f.k === "inside")) out.inside.push({ rule: norm, by: out.rules[found.find((f) => f.k === "inside")!.j] });
    else {
      push();
      const p = found.find((f) => f.k === "partial");
      if (p) out.partial.push({ rule: norm, by: out.rules[p.j] });
    }
  }
  return out;
}
