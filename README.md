# Backuper

Windows backup app for copying between drives on the same PC (an alternative to Cobian Reflector).
Hebrew RTL UI, dark mode, built-in scheduler, runs from the tray and starts with Windows.

Backups are **plain files and folders**: no private format, no compression, no encryption. You can open a backup straight in Explorer.

## How backups work

A task has **one or more source folders**, one destination folder and a schedule. All sources of a task run together and their backup folders share the same date. Each source has its own chain of dated folders in the destination, named after the source folder (editable):

```
E:\Backups\Documents 2026-10-01 03-00 מלא            <- full: identical to the source
E:\Backups\Documents 2026-10-02 03-00 אינקרמנטלי     <- only files new/changed since the previous backup
E:\Backups\Pictures  2026-10-01 03-00 מלא
```

- **Full**: a new dated folder identical to the source (files deleted from the source don't show up).
  - *Fast full* (optional, keepCount = 1): renames the previous full folder to the new date and mirrors into it, so only the differences get copied.
- **Incremental**: every run creates a **new** dated folder holding only the files that are new or changed since the previous backup (compared against the full backup plus the incrementals after it, by size and modified time). If nothing changed, the folder stays empty, so Explorer still shows the backup ran. The first backup is always full. Optional "new full every N days".
- **Retention**: a full backup plus the incrementals after it is a chain. When a full backup completes, chains beyond `keepCount` (default 1) are deleted, together with empty incremental folders and incomplete ones.
- **Delete before** (optional, per task): deletes the old backups *before* the full backup starts, to free disk space. If that backup then fails, no previous backup is left (the UI warns about this). By default old backups are deleted only after the new one succeeds.
- A folder still being written, or one that failed or was cancelled, carries a `.partial` suffix and is never used as a full base.
- Empty folders inside the source aren't copied unless "copy empty folders" is on (full backups only).

**Filter rules** (what never gets backed up), per task and/or global (Settings): file extension, file-name wildcard (`~$*`), folder name or path (`node_modules`), files larger than N MB, files not modified for N days, hidden files, system files. `$RECYCLE.BIN` and `System Volume Information` are always skipped. Full and incremental backups apply the same rules (both come from robocopy).

**Schedules**: manual, one-time, daily, weekly (chosen weekdays), monthly (day 1-31, clamped to the month's end), every X minutes/hours. A run missed while the PC was off or asleep can be caught up on next start ("catch-up", per task).

Also included: a run log with per-source details, errors and the full log; a toast notification on success/failure; bulk edit of several tasks (choose fields, preview the diff, apply, undo); sorting the task list by name, source or destination folder (grouped by folder); a backups manager per task (per source: list, type, size, open, delete); folders picked with the Windows folder dialog (several sources at once).

## Use

Install with `src-tauri\target\release\bundle\nsis\Backuper_<version>_x64-setup.exe` (per-user install, no admin needed).
On first launch the app registers itself to start with Windows (to the tray, `--hidden`). You can change this in Settings.
Closing the window minimizes to the tray. Right-click the tray icon to pause scheduling or exit.

App data lives in `%APPDATA%\org.tovtech.backuper\`: `tasks.json`, `state.json` (next/last run), `settings.json`, `history.json`, and `logs\` (robocopy logs per run).

## Develop

Requirements: Node 20+, Rust (MSVC toolchain), Visual Studio Build Tools, WebView2 (built into Windows 10/11).

```
npm install
npm run app:dev        # Tauri dev (hot reload)
npm run app:build      # release exe + NSIS installer
npm run dev            # UI only in a browser, with fake data (src/lib/devMock.ts)
cd src-tauri && cargo test   # unit tests + end-to-end tests against real robocopy
```

## Not done yet

- Running as a Windows service (backups without a logged-in user). The core (`core.rs`) doesn't depend on the UI, so it can be moved into a service process later.
