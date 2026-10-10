# Backuper

Windows backup app for copying between drives on the same PC (an alternative to Cobian Reflector).
Hebrew RTL UI, dark mode, built-in scheduler, runs from the tray and starts with Windows.

Backups are **plain files and folders**: no private format, no compression, no encryption. You can open a backup straight in Explorer.

**Download:** https://star69995.github.io/backuper/ (Hebrew download page with screenshots and every version), or the [GitHub releases](https://github.com/Star69995/backuper/releases). Versions after 0.3.4 update themselves (see Self-update below); older ones update by running the newer installer once.

## How backups work

A task has **one or more source folders**, one destination folder and a schedule. All sources of a task run together and their backup folders share the same date. Each source has its own chain of dated folders in the destination, named after the source folder (editable):

```
E:\Backups\Documents 2026-10-01 03-00 מלא            <- full: identical to the source
E:\Backups\Documents 2026-10-02 03-00 אינקרמנטלי     <- only files new/changed since the previous backup
E:\Backups\Pictures  2026-10-01 03-00 מלא
```

- **Full**: a new dated folder identical to the source (files deleted from the source don't show up).
  - *Fast full* (optional, keepCount = 1): renames the previous full folder to the new date and mirrors into it, so only the differences get copied.
- **Incremental**: every run creates a **new** dated folder holding only the files that are new or changed since the previous backup (compared against the full backup plus the incrementals after it, by size and modified time). The app scans the source itself (one directory read per folder, no file is opened) with the same filter semantics as robocopy, and reads what is already backed up from a hidden `.backuper-index` file in each completed backup folder instead of walking the destination drive (a folder without one, e.g. from an older version, is walked once and gets one). Measured on 200,000 files (1% changed), source and backups on two hard disks: change detection went from ~32s to ~0.8s, the whole incremental from ~34s to ~3.3s. Files deleted by hand from a backup folder still count as backed up until the next full. If nothing changed, the folder stays empty, so Explorer still shows the backup ran. The first backup is always full.
- **Combined**: full backups on their own schedule (e.g. weekly on Friday) and incrementals on another (e.g. daily). When both are due at once, only the full runs. Each full starts a new chain.
- **Retention** (`keepMode`): a full backup plus the incrementals after it is a chain. Three modes:
  - *by count* (default) - when a full backup completes, chains beyond `keepCount` (default 1) are deleted.
  - *by time* - keeps whatever is needed to restore any moment of the last `keepDays` days: a chain is deleted only once the next full is also older than that. Checked after every backup, full or incremental.
  - *forever* - chains are never deleted (old backups are removed by hand in "manage backups"). Fast full backup and delete-before don't apply.

  Incomplete folders are always deleted, and (option "delete empty incremental folders", on by default) a full backup also deletes incremental folders that ended up empty. Cobian tasks with unlimited copies import as *forever*.
- **Sizes in the result**: after a full backup, or an incremental that copied something, the run message (and the Windows notification) gives the size of the new backup folder, and when older backups were deleted, how much space that freed (also per deleted folder). The run details show both (`backupBytes`, `freedBytes`).
- **Delete before** (optional, per task): deletes the old backups *before* the full backup starts, to free disk space. If that backup then fails, no previous backup is left (the UI warns about this). By default old backups are deleted only after the new one succeeds.
- A folder still being written, or one that failed or was cancelled, carries a `.partial` suffix and is never used as a full base.
- Empty folders inside the source aren't copied unless "copy empty folders" is on (full backups only).

**Filter rules**, per task and/or global (Settings). Rule names say what they do: "לגבות רק סוגי קבצים" (`docx, xlsx`; plain extensions, though `*.lrcat` and exact names like `notes.txt` still work) limits the backup to those files. "ביטוי רגולרי (מתקדם)" matches the file name (case-insensitive) and either skips or keeps only matching files. Every other rule excludes: file extension, file-name wildcard (`~$*`), folder name or path (`node_modules`), files larger than N MB, files not modified for N days, hidden files, system files. `$RECYCLE.BIN` and `System Volume Information` are always skipped. Full and incremental backups apply the same rules: the incremental scan reproduces robocopy's matching (DOS wildcards on the long and the 8.3 name, `/XJ`, `/XA`, `/MAX`, `/MAXAGE`), checked by a test against real robocopy. robocopy has no regex, so for a full backup with a regex rule the source is listed once first and the result is handed to robocopy as plain file names (`native::resolve_regex`; fails with an explanation if the list wouldn't fit the command line).

**Schedules**: manual, one-time, daily, weekly (chosen weekdays), monthly (day 1-31, clamped to the month's end), every X minutes/hours. A run missed while the PC was off or asleep can be caught up on next start ("catch-up", per task).

**When a drive connects** (per task, on top of the schedule): off / back up automatically / ask first. Meant for an external disk or USB stick: when all the drives of a task (its destination and sources, e.g. `E:` and `C:`) are available after one of them wasn't, the task is queued right away, or the window pops up asking "back up now?" (several tasks can be answered in one dialog). Nothing fires at startup for drives that are already connected, nor while scheduling is paused. Only drive-letter paths are watched (not `\server\share`). The drive is recognized by its letter, so another disk that gets the same letter also counts.

**Import from Cobian**: "ייבוא וייצוא" - import from a file reads a Cobian Backup / Reflector task list (`.lst`) and shows a preview first. Name, enabled state, local sources/destination, schedule, full/incremental, a fixed weekly full day (imported as combined mode), copies to keep, empty folders, and include/exclude masks all carry over. Anything without an equivalent gets a per-task warning (size/date/path include filters, "full every N backups", differential, FTP sources). Tasks keep Cobian's task id, so importing again offers to update them instead of duplicating them. The app doesn't recognize Cobian's existing backup folders, so each task's first run is a full backup.

**Import from a simple JSON config**: a file with `copy_sources` (groups of folders) and `copy_destinations` imports as one manual task: all groups merged, each folder once, first destination (warnings say what was merged or skipped). Importing again updates that task.

**Task list backup** (Settings - "גיבוי רשימת המשימות"): every change to the task list saves a dated snapshot in the app data dir (the last 50), and any snapshot can be restored through the same preview (deleted tasks are pre-checked, changed ones can be checked to revert them; tasks created later are kept). Optionally, the same dated snapshots (the last 50) are also kept in a folder of your choice (e.g. on the backup drive), in a hidden subfolder `Backuper - גיבויי רשימת משימות`, so the list survives the PC itself; "שחזור מתיקייה" lists and restores the snapshots in any folder (e.g. after reinstalling). Snapshot files and folders are hidden + system (files also read-only), so they don't show in Explorer and aren't deleted by accident - this guards against mistakes, not against someone deliberately removing them. If `tasks.json` is damaged (or missing while snapshots exist), it's renamed to `tasks - פגום <date>.json`, the newest snapshot (from the app data dir or the copy folder) is loaded, and a notice explains what happened (also as a Windows notification when the app starts hidden). An empty list saved on purpose stays empty. **Export** saves all tasks, or the selected ones, to a `.json` file; importing that file (on this or another PC) goes through the same preview. Tasks keep their ids, so importing again updates instead of duplicating.

**Self-update** (Settings - "עדכוני תוכנה"): the app checks the GitHub releases for a newer version a minute after it starts and then every 6 hours. *Automatic* (default) downloads it and installs it only while the window is closed (in the tray) and no backup is running or queued; the installer runs in passive mode, the app restarts (back into the tray) and shows a "updated to version X" notification. *Notify only* shows a notification and installs on "עדכון עכשיו". *Off* checks only on demand. "עדכון עכשיו" during a backup waits until the running and queued backups are done. A newer version also shows a button at the bottom of the side nav. Every update is verified against the app's signing key before it installs.

**Notification sounds** (Settings - "התראות"): Windows notifications play a sound - one for success (also used when a backup starts on drive connect) and one for failures, warnings and notices. Each can be a Windows toast sound or silent, with a button that shows a sample notification. Windows' own notification settings and Do Not Disturb still apply.

Also included: an in-app help page ("עזרה") for end users - what happens in a run and the copy engine, backup types, restoring files, scheduling, drive connect, filters, managing tasks, errors and an FAQ, with search; a run log with per-source details, errors and the full log (failed files are grouped by cause - e.g. in use by another program, deleted during the backup, no permission, disk full - each with why it happened and what to do); a toast notification on success/failure; bulk edit of several tasks (choose fields, preview the diff, apply, undo); sorting the task list by name, next scheduled run (tasks with none last), source or destination folder (grouped by folder); resizable task table columns, including the task name (drag a header border, double-click to reset; remembered per machine). The name column fills the free space until a border is first dragged; a backups manager per task (per source: list, type, size, open, delete); folders picked with the Windows folder dialog (several sources at once).

## Use

Install with `src-tauri\target\release\bundle\nsis\Backuper_<version>_x64-setup.exe` (per-user install into `%LOCALAPPDATA%\Backuper`, no admin needed, so the self-updater never triggers a UAC prompt). The installer is in Hebrew. Running a newer installer over an installed version updates it in place (tasks, settings and the log are kept).
On first launch the app registers itself to start with Windows (to the tray, `--hidden`). You can change this in Settings.
Closing the window minimizes to the tray. Right-click the tray icon to pause scheduling or exit.

App data lives in `%APPDATA%\org.tovtech.backuper\`: `tasks.json`, `state.json` (next/last run), `settings.json`, `history.json`, `logs\` (robocopy logs per run) and `task-list-backups\` (task list snapshots).

## Develop

Requirements: Node 20+, Rust (MSVC toolchain), Visual Studio Build Tools, WebView2 (built into Windows 10/11).

```
npm install
npm run app:dev        # Tauri dev (hot reload)
npm run app:build      # release exe + NSIS installer
npm run dev            # UI only in a browser, with fake data (src/lib/devMock.ts)
npm run bump           # version +0.0.1 everywhere (or: npm run bump -- minor / major / 1.2.3)
npm run release        # signed build + GitHub release with latest.json (notes from commits, asks first; -- notes.md = own notes, --dry = build only)
cd src-tauri && cargo test   # unit tests + end-to-end tests against real robocopy
```

**Releasing**: `npm run bump`, commit and push, then `npm run release` (or the npm scripts button in VS Code). Without a notes file it makes the notes from the commit subjects since the last release (docs/chore left out) and asks before publishing; `npm run release -- notes.md` uses your own notes. The script builds with the updater signing key (`TAURI_SIGNING_PRIVATE_KEY`, or `~/.tauri/backuper.key`), writes `latest.json` (version, notes, signature, installer URL) and publishes both with the installer as release `v<version>`. Installed apps read `https://github.com/Star69995/backuper/releases/latest/download/latest.json`. The public key is in `tauri.conf.json` (`plugins.updater.pubkey`); keep the private key backed up - without it, installed copies can't be updated anymore.

**Website** (`site/`): the landing/download page at https://star69995.github.io/backuper/ - plain static HTML/CSS/JS, no build step. `.github/workflows/pages.yml` deploys it to GitHub Pages on every push to `master` that touches `site/`. The version list and the download buttons are filled in the browser from the GitHub Releases API (`site/app.js`: the `*-setup.exe` asset of each non-draft release, notes, SHA-256), so publishing a release needs no site change. Preview locally with `python -m http.server 5240 --directory site`. Screenshots in `site/img/` are taken from `npm run dev` (the mock data in `src/lib/devMock.ts`) at 1180x740, scale 1.5, WebP; `og.png` is a 1200x630 capture of the page itself.

## Not done yet

- Running as a Windows service (backups without a logged-in user). The core (`core.rs`) doesn't depend on the UI, so it can be moved into a service process later.

## License

Copyright (C) 2026 Star

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 3 of the License only. See [LICENSE](LICENSE) for the full text.
