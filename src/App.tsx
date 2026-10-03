import { CircleHelp, Download, HardDriveDownload, History, ListChecks, Pause, Play, Settings as SettingsIcon } from "lucide-react";
import { type ReactNode, useState } from "react";
import { api } from "./api";
import DrivePromptDialog from "./components/DrivePromptDialog";
import HelpView from "./components/HelpView";
import HistoryView from "./components/HistoryView";
import NoticeDialog from "./components/NoticeDialog";
import SettingsView from "./components/SettingsView";
import TasksView from "./components/TasksView";
import { cx, Spinner } from "./components/ui";
import { useSnapshot, useTheme } from "./lib/useSnapshot";

type View = "tasks" | "history" | "settings" | "help";

export default function App() {
  const { snap, refresh } = useSnapshot();
  const [view, setView] = useState<View>("tasks");
  useTheme(snap?.settings.theme);

  if (!snap) {
    return (
      <div className="flex h-full items-center justify-center text-muted">
        <Spinner />
      </div>
    );
  }

  const paused = snap.settings.schedulerPaused;
  const togglePause = () => api.saveSettings({ ...snap.settings, schedulerPaused: !paused }).then(refresh);

  const u = snap.update;
  const updateFound = u.version !== null && ["available", "downloading", "ready"].includes(u.state);

  const nav: { id: View; label: string; icon: ReactNode }[] = [
    { id: "tasks", label: "משימות", icon: <ListChecks size={18} /> },
    { id: "history", label: "יומן ריצות", icon: <History size={18} /> },
    { id: "settings", label: "הגדרות", icon: <SettingsIcon size={18} /> },
    { id: "help", label: "עזרה", icon: <CircleHelp size={18} /> },
  ];

  return (
    <div className="flex h-full flex-col md:flex-row">
      <aside className="flex shrink-0 flex-row items-center gap-1 border-b border-line bg-panel px-2 py-2 md:w-56 md:flex-col md:items-stretch md:border-b-0 md:border-l md:px-3 md:py-4">
        <div className="me-2 flex items-center gap-2 px-1 md:me-0 md:mb-5 md:px-2">
          <div className="flex size-8 items-center justify-center rounded-lg bg-accent text-white">
            <HardDriveDownload size={18} />
          </div>
          <div className="hidden leading-tight sm:block">
            <div className="text-[15px] font-semibold">Backuper</div>
            <div className="text-xs text-muted">גיבוי קבצים</div>
          </div>
        </div>
        <nav className="flex flex-1 flex-row gap-1 md:flex-none md:flex-col">
          {nav.map((n) => (
            <button
              key={n.id}
              type="button"
              onClick={() => setView(n.id)}
              className={cx(
                "flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm font-medium transition-colors",
                view === n.id ? "bg-accent-soft text-accent" : "text-muted hover:bg-hover hover:text-fg",
              )}
            >
              {n.icon}
              <span className="hidden sm:inline">{n.label}</span>
            </button>
          ))}
        </nav>
        <div className="flex gap-1 md:mt-auto md:flex-col">
          {updateFound && (
            <button
              type="button"
              onClick={() => setView("settings")}
              title={`גרסה ${u.version} זמינה - לפרטים ולעדכון`}
              className="flex w-full items-center gap-2 rounded-lg border border-accent/40 bg-accent-soft px-2.5 py-2 text-start text-[13px] text-accent transition-colors hover:bg-hover"
            >
              <Download size={16} />
              <span className="hidden flex-col leading-tight md:flex">
                <span className="font-medium">גרסה {u.version} זמינה</span>
                <span className="text-xs">{u.installWaiting ? "תותקן בסיום הגיבוי" : "לחצו לעדכון"}</span>
              </span>
            </button>
          )}
          <button
            type="button"
            onClick={togglePause}
            title={paused ? "חידוש התזמון" : "השהיית התזמון"}
            className={cx(
              "flex w-full items-center gap-2 rounded-lg border px-2.5 py-2 text-start text-[13px] transition-colors",
              paused ? "border-warn/40 bg-warn-soft text-warn" : "border-line text-muted hover:bg-hover",
            )}
          >
            {paused ? <Play size={16} /> : <Pause size={16} />}
            <span className="hidden flex-col leading-tight md:flex">
              <span className="font-medium text-fg">{paused ? "התזמון מושהה" : "התזמון פעיל"}</span>
              <span className="text-xs">{paused ? "לחצו לחידוש" : "לחצו להשהיה"}</span>
            </span>
          </button>
        </div>
      </aside>
      <main className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        {view === "tasks" && <TasksView snap={snap} refresh={refresh} />}
        {view === "history" && <HistoryView snap={snap} />}
        {view === "settings" && <SettingsView snap={snap} refresh={refresh} />}
        {view === "help" && <HelpView />}
      </main>
      <DrivePromptDialog snap={snap} />
      <NoticeDialog snap={snap} refresh={refresh} />
    </div>
  );
}
