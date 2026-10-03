// Bumps the app version in every file that holds it, so they never drift.
// Usage: npm run bump [-- patch|minor|major|X.Y.Z]   (default: patch)
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, dirname } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// Each pattern captures everything before the quoted version as group 1.
const targets = [
  ["package.json", [/^(  "version": )"[^"]*"/m]],
  [
    "package-lock.json",
    [/^(  "version": )"[^"]*"/m, /("packages": \{\s*"": \{[^}]*?"version": )"[^"]*"/],
  ],
  ["src-tauri/tauri.conf.json", [/^(  "version": )"[^"]*"/m]],
  ["src-tauri/Cargo.toml", [/(\[package\][^[]*?^version = )"[^"]*"/m]],
  ["src-tauri/Cargo.lock", [/(name = "backuper"\r?\nversion = )"[^"]*"/]],
];

const current = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const arg = process.argv[2] ?? "patch";

function next(version, kind) {
  if (/^\d+\.\d+\.\d+$/.test(kind)) return kind;
  const [major, minor, patch] = version.split(".").map(Number);
  if (kind === "major") return `${major + 1}.0.0`;
  if (kind === "minor") return `${major}.${minor + 1}.0`;
  if (kind === "patch") return `${major}.${minor}.${patch + 1}`;
  throw new Error(`Unknown bump "${kind}" - use patch, minor, major or X.Y.Z`);
}

const version = next(current, arg);

// Compute everything first, write only if all patterns matched.
const updates = targets.map(([file, patterns]) => {
  const path = join(root, file);
  let text = readFileSync(path, "utf8");
  for (const pattern of patterns) {
    if (!pattern.test(text)) throw new Error(`Version not found in ${file} (${pattern})`);
    text = text.replace(pattern, `$1"${version}"`);
  }
  return [path, text];
});
for (const [path, text] of updates) writeFileSync(path, text);

console.log(`${current} -> ${version}`);
