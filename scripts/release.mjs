// Builds the signed installer and publishes it as a GitHub release, with the latest.json
// the in-app updater reads (https://github.com/<repo>/releases/latest/download/latest.json).
// Usage: npm run release [-- <notes.md>] [--dry] [--yes]
//   notes.md = the release notes (GitHub release body, also shown in the app's update card).
//              Without it, the notes are made from the commit subjects since the last release.
//   --dry    = build and write latest.json, don't publish.
//   --yes    = don't ask before publishing (needed when not run from a terminal).
// Run `npm run bump` (and commit + push) first: the release is tagged at the pushed HEAD.
// Signing key: $TAURI_SIGNING_PRIVATE_KEY (key or path), else ~/.tauri/backuper.key.
import { execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline/promises";
import { fileURLToPath } from "node:url";

const REPO = "Star69995/backuper";
const USAGE = "Usage: npm run release [-- <notes.md>] [--dry] [--yes]";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const run = (cmd, opts = {}) => execSync(cmd, { cwd: root, stdio: "inherit", ...opts });
const out = (cmd) => execSync(cmd, { cwd: root, encoding: "utf8" }).trim();
const fail = (msg) => {
  console.error(`\n${msg}`);
  process.exit(1);
};

const args = process.argv.slice(2);
const dry = args.includes("--dry");
const yes = args.includes("--yes");
const notesArg = args.find((a) => !a.startsWith("--"));
if (notesArg && !existsSync(notesArg)) fail(`Notes file not found: ${notesArg}\n${USAGE}`);

const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const tag = `v${version}`;

if (!dry) {
  const dirty = out("git status --porcelain");
  if (dirty) fail(`Commit (or stash) your changes first - the release is tagged at HEAD:\n${dirty}`);
  const head = out("git rev-parse HEAD");
  let pushed = "";
  try {
    pushed = out("git rev-parse @{u}");
  } catch {}
  if (head !== pushed) fail("Push first - HEAD isn't on the remote branch.");
  let exists = true;
  try {
    out(`gh release view ${tag} --repo ${REPO}`);
  } catch {
    exists = false;
  }
  if (exists) fail(`Release ${tag} already exists - run \`npm run bump\` first.`);
}

/** "- subject" for each commit since the last published release (docs/chore/style and the like left out). */
function notesFromCommits() {
  let since = "";
  try {
    const last = out(`gh release view --repo ${REPO} --json tagName -q .tagName`);
    out("git fetch --tags -q");
    since = `${last}..HEAD`;
  } catch {}
  const lines = out(`git log ${since} --no-merges --format=%s`)
    .split("\n")
    .filter((s) => s && !/^(docs|chore|style|ci|build|test)(\(.*?\))?!?:/i.test(s))
    .map((s) => s.replace(/^\w+(\(.*?\))?!?:\s*/, ""))
    .map((s) => `- ${s.charAt(0).toUpperCase()}${s.slice(1)}`);
  return lines.length ? lines.join("\n") : "- Small fixes";
}

const notes = notesArg
  ? readFileSync(notesArg, "utf8").trim()
  : `Windows installer (x64). Upgrading installs in place over the previous version.\n\n${notesFromCommits()}`;

console.log(`\nRelease ${tag}${dry ? " (dry run)" : ""} - notes:\n\n${notes}\n`);
if (!dry && !yes) {
  if (!process.stdin.isTTY) fail("Not a terminal - pass --yes to publish without asking.");
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  const answer = await rl.question(`Build and publish ${tag} with these notes? [y/N] `);
  rl.close();
  if (!/^y(es)?$/i.test(answer.trim())) fail("Cancelled. To write the notes yourself: npm run release -- notes.md");
}

const keyPath = join(homedir(), ".tauri", "backuper.key");
const env = { ...process.env };
if (!env.TAURI_SIGNING_PRIVATE_KEY) {
  if (!existsSync(keyPath)) fail(`No signing key: set TAURI_SIGNING_PRIVATE_KEY or put the key at ${keyPath}.`);
  env.TAURI_SIGNING_PRIVATE_KEY = keyPath;
}
env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";

run("npm run app:build", { env });

const bundleDir = join(root, "src-tauri", "target", "release", "bundle", "nsis");
const exeName = `Backuper_${version}_x64-setup.exe`;
const exe = join(bundleDir, exeName);
const sig = `${exe}.sig`;
if (!existsSync(exe) || !existsSync(sig)) fail(`Missing ${exe} or its .sig - did the build sign it?`);

const platform = {
  signature: readFileSync(sig, "utf8").trim(),
  url: `https://github.com/${REPO}/releases/download/${tag}/${exeName}`,
};
const latest = {
  version,
  notes,
  pub_date: new Date().toISOString(),
  platforms: { "windows-x86_64-nsis": platform, "windows-x86_64": platform },
};
const latestPath = join(bundleDir, "latest.json");
writeFileSync(latestPath, JSON.stringify(latest, null, 2) + "\n");
const notesPath = join(bundleDir, "release-notes.md");
writeFileSync(notesPath, notes + "\n");
console.log(`\nWrote ${latestPath}`);

if (dry) {
  console.log("Dry run - nothing published.");
} else {
  run(
    `gh release create ${tag} "${exe}" "${latestPath}" --repo ${REPO} --target ${out("git rev-parse HEAD")} --title "Backuper ${version}" --notes-file "${notesPath}"`,
  );
  console.log(`\nPublished ${tag}. Installed copies pick it up on their next update check.`);
}
