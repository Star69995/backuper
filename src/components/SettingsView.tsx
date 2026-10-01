import { getVersion } from "@tauri-apps/api/app";
import { Bell, Filter, Monitor, Moon, Power, Sun } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { api, errorText } from "../api";
import type { Settings, Snapshot } from "../types";
import { useFeedback } from "./feedback";
import FilterRulesEditor from "./FilterRulesEditor";
import { Button, Segmented, Toggle } from "./ui";

function Card({ title, icon, children }: { title: string; icon: ReactNode; children: ReactNode }) {
  return (
    <section className="rounded-xl border border-line bg-panel p-5">
      <h2 className="mb-4 flex items-center gap-2 font-semibold">
        <span className="text-muted">{icon}</span>
        {title}
      </h2>
      <div className="flex flex-col gap-5">{children}</div>
    </section>
  );
}

export default function SettingsView({ snap, refresh }: { snap: Snapshot; refresh: () => void }) {
  const { toast } = useFeedback();
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion);
  }, []);

  const s = snap.settings;
  const update = (patch: Partial<Settings>) =>
    api
      .saveSettings({ ...s, ...patch })
      .then(refresh)
      .catch((e) => {
        toast({ tone: "bad", title: "השמירה נכשלה", message: errorText(e) });
        throw e;
      });

  // Filter rules are edited as a draft and saved explicitly (a new rule starts empty).
  const [filters, setFilters] = useState(s.globalFilters);
  const filtersDirty = JSON.stringify(filters) !== JSON.stringify(s.globalFilters);

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-4 p-4 md:p-6">
      <header>
        <h1 className="text-xl font-semibold">הגדרות</h1>
      </header>

      <Card title="מראה" icon={<Sun size={18} />}>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <span className="text-sm font-medium">ערכת צבעים</span>
          <Segmented
            value={s.theme}
            onChange={(theme) => update({ theme })}
            options={[
              { value: "system", label: "לפי Windows", icon: <Monitor size={14} /> },
              { value: "light", label: "בהיר", icon: <Sun size={14} /> },
              { value: "dark", label: "כהה", icon: <Moon size={14} /> },
            ]}
          />
        </div>
      </Card>

      <Card title="הפעלה ורקע" icon={<Power size={18} />}>
        <Toggle
          checked={snap.autostart}
          onChange={(v) =>
            api
              .setAutostart(v)
              .then(refresh)
              .catch((e) => toast({ tone: "bad", title: "השינוי נכשל", message: errorText(e) }))
          }
          label="הפעלה עם הדלקת המחשב"
          description="התוכנה עולה אוטומטית לאזור ההודעות (ליד השעון) ומתחילה לבצע את התזמונים."
        />
        <Toggle
          checked={s.closeToTray}
          onChange={(closeToTray) => update({ closeToTray })}
          label="סגירת החלון ממזערת לאזור ההודעות"
          description="התזמון ממשיך לפעול ברקע. ליציאה מלאה: לחיצה ימנית על הסמל ליד השעון - יציאה. אם האפשרות כבויה, סגירת החלון עוצרת את התזמון."
        />
        <Toggle
          checked={s.schedulerPaused}
          onChange={(schedulerPaused) => update({ schedulerPaused })}
          label="השהיית כל התזמונים"
          description="משימות לא ירוצו אוטומטית עד לחידוש. אפשר עדיין להריץ ידנית."
        />
      </Card>

      <Card title="כללי סינון כלליים" icon={<Filter size={18} />}>
        <p className="-mt-2 text-[13px] text-muted">
          קבצים שתואמים לכללים האלה לא יגובו באף משימה (אלא אם כיבו במשימה את "החלת כללי הסינון הכלליים").
        </p>
        <FilterRulesEditor rules={filters} onChange={setFilters} emptyText="אין כללים כלליים." />
        {filtersDirty && (
          <div className="flex justify-end gap-2">
            <Button size="sm" onClick={() => setFilters(s.globalFilters)}>
              ביטול שינויים
            </Button>
            <Button
              size="sm"
              variant="primary"
              onClick={() =>
                update({ globalFilters: filters })
                  .then(() => toast({ tone: "ok", title: "כללי הסינון נשמרו" }))
                  .catch(() => {})
              }
            >
              שמירת כללים
            </Button>
          </div>
        )}
      </Card>

      <Card title="התראות" icon={<Bell size={18} />}>
        <Toggle checked={s.notifySuccess} onChange={(notifySuccess) => update({ notifySuccess })} label="התראה כשגיבוי מצליח" />
        <Toggle checked={s.notifyFailure} onChange={(notifyFailure) => update({ notifyFailure })} label="התראה כשגיבוי נכשל" />
      </Card>

      <p className="text-center text-xs text-muted">Backuper {version} - מנוע העתקה: robocopy</p>
    </div>
  );
}
