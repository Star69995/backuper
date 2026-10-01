# Backuper

Windows backup app for copying between drives on the same PC (an alternative to Cobian Reflector).
Hebrew RTL UI, dark mode, built-in scheduler, runs from the tray and starts with Windows.

Backups are **plain files and folders**: no private format, no compression, no encryption. You can open a backup straight in Explorer.

## How backups work

Each task has a source, a destination root and a schedule. Backups go into dated folders inside the destination:

```
E:\Backups\Documents 2026-10-01 03-00\...   <- date the backup folder was created
```

- **Full**: creates a new dated folder that is identical to the source (files deleted from the source don't show up). After it succeeds, older backups are deleted (keeps the newest `keepCount`, default 1).
  - *Fast full* (optional, only with keepCount = 1): renames the previous folder to the new date and mirrors into it, so only the differences get copied.
- **Incremental**: copies only new or changed files into the latest complete dated folder. Nothing is deleted. If there's no previous backup yet, it runs a full backup. Optional "new full every N days".
- A full backup that is still running or that failed carries a `.partial` suffix. It is never used as an incremental base, and it is cleaned up by the next successful full.
- Empty folders aren't copied unless the task's "copy empty folders" option is on.

**Filter rules** (what never gets backed up), per task and/or global (Settings): file extension, file-name wildcard (`~$*`), folder name or path (`node_modules`), files larger than N MB, files not modified for N days, hidden files, system files. `$RECYCLE.BIN` and `System Volume Information` are always skipped.

**Schedules**: manual, one-time, daily, weekly (chosen weekdays), monthly (day 1-31, clamped to the month's end), every X minutes/hours. A run missed while the PC was off or asleep can be caught up on next start ("catch-up", per task).

Also included: run log with per-run details, robocopy errors and the full log; a toast notification on success/failure; bulk edit of several tasks (choose fields, preview the diff, apply, undo); sorting the task list by name, source or destination folder (grouped by folder); a per-task backup manager (list, size, open, delete).

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
