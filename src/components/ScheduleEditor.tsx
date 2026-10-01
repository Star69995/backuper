import { CalendarClock } from "lucide-react";
import { useEffect, useState } from "react";
import { api, errorText } from "../api";
import { fmtSmart, WEEKDAYS, WEEKDAYS_SHORT } from "../lib/format";
import type { Schedule, ScheduleKind } from "../types";
import { cx, Field, NumberInput, Segmented, Select, TextInput } from "./ui";

const KINDS: { value: ScheduleKind; label: string }[] = [
  { value: "manual", label: "ידני" },
  { value: "once", label: "חד-פעמי" },
  { value: "daily", label: "יומי" },
  { value: "weekly", label: "שבועי" },
  { value: "monthly", label: "חודשי" },
  { value: "interval", label: "כל X דקות" },
];

function timeOf(s: Schedule) {
  return "time" in s ? s.time : "03:00";
}

function tomorrowAt(time: string) {
  const d = new Date(Date.now() + 86_400_000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${time}`;
}

export function defaultSchedule(kind: ScheduleKind, prev: Schedule): Schedule {
  const time = timeOf(prev);
  switch (kind) {
    case "manual":
      return { kind };
    case "once":
      return { kind, at: tomorrowAt(time) };
    case "daily":
      return { kind, time };
    case "weekly":
      return { kind, days: [0], time };
    case "monthly":
      return { kind, day: 1, time };
    case "interval":
      return { kind, minutes: 60 };
  }
}

export default function ScheduleEditor({ value, onChange }: { value: Schedule; onChange: (s: Schedule) => void }) {
  const [preview, setPreview] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (value.kind === "manual") {
      setPreview([]);
      setError(null);
      return;
    }
    const t = setTimeout(() => {
      api
        .previewSchedule(value)
        .then((p) => {
          setPreview(p);
          setError(null);
        })
        .catch((e) => {
          setPreview([]);
          setError(errorText(e));
        });
    }, 250);
    return () => clearTimeout(t);
  }, [value]);

  const [unit, setUnit] = useState<"minutes" | "hours">(
    value.kind === "interval" && value.minutes >= 60 && value.minutes % 60 === 0 ? "hours" : "minutes",
  );

  return (
    <div className="flex flex-col gap-4">
      <Segmented value={value.kind} onChange={(k) => onChange(defaultSchedule(k, value))} options={KINDS} />

      {value.kind === "manual" && (
        <p className="text-[13px] text-muted">המשימה תרוץ רק כשתפעילו אותה בעצמכם (כפתור "הרץ עכשיו").</p>
      )}

      {value.kind === "once" && (
        <Field label="תאריך ושעה" className="max-w-64">
          <TextInput type="datetime-local" dir="ltr" value={value.at} onChange={(e) => onChange({ ...value, at: e.target.value })} />
        </Field>
      )}

      {value.kind === "daily" && (
        <Field label="שעה" className="max-w-40">
          <TextInput type="time" dir="ltr" value={value.time} onChange={(e) => onChange({ ...value, time: e.target.value })} />
        </Field>
      )}

      {value.kind === "weekly" && (
        <div className="flex flex-wrap items-end gap-4">
          <div className="flex flex-col gap-1.5">
            <span className="text-[13px] font-medium">ימים</span>
            <div className="flex gap-1">
              {WEEKDAYS_SHORT.map((d, i) => {
                const on = value.days.includes(i);
                return (
                  <button
                    key={i}
                    type="button"
                    title={WEEKDAYS[i]}
                    aria-pressed={on}
                    onClick={() =>
                      onChange({ ...value, days: on ? value.days.filter((x) => x !== i) : [...value.days, i].sort() })
                    }
                    className={cx(
                      "size-9 rounded-lg border text-[13px] font-medium transition-colors",
                      on ? "border-accent bg-accent text-white" : "border-line bg-panel hover:bg-hover",
                    )}
                  >
                    {d}
                  </button>
                );
              })}
            </div>
          </div>
          <Field label="שעה" className="w-36">
            <TextInput type="time" dir="ltr" value={value.time} onChange={(e) => onChange({ ...value, time: e.target.value })} />
          </Field>
        </div>
      )}

      {value.kind === "monthly" && (
        <div className="flex flex-wrap items-start gap-4">
          <Field label="יום בחודש" hint="בחודש קצר יותר - ביום האחרון שלו" className="w-48">
            <NumberInput min={1} max={31} value={value.day} onChange={(day) => onChange({ ...value, day })} />
          </Field>
          <Field label="שעה" className="w-36">
            <TextInput type="time" dir="ltr" value={value.time} onChange={(e) => onChange({ ...value, time: e.target.value })} />
          </Field>
        </div>
      )}

      {value.kind === "interval" && (
        <div className="flex items-end gap-2">
          <Field label="כל" className="w-28">
            <NumberInput
              min={1}
              value={unit === "hours" ? value.minutes / 60 : value.minutes}
              onChange={(n) => onChange({ ...value, minutes: Math.max(1, unit === "hours" ? n * 60 : n) })}
            />
          </Field>
          <Select
            className="w-28"
            value={unit}
            onChange={(e) => {
              const u = e.target.value as "minutes" | "hours";
              setUnit(u);
              // Keep the number the user typed; only its unit changes.
              onChange({ ...value, minutes: u === "hours" ? value.minutes * 60 : Math.max(1, Math.round(value.minutes / 60)) });
            }}
          >
            <option value="minutes">דקות</option>
            <option value="hours">שעות</option>
          </Select>
        </div>
      )}

      {value.kind !== "manual" && (
        <div className="flex items-start gap-2 rounded-lg bg-panel2 px-3 py-2 text-[13px]">
          <CalendarClock size={16} className="mt-0.5 shrink-0 text-muted" />
          {error ? (
            <span className="text-bad">{error}</span>
          ) : preview.length === 0 ? (
            <span className="text-muted">אין ריצות עתידיות (המועד כבר עבר)</span>
          ) : (
            <span>
              <span className="text-muted">הריצות הבאות: </span>
              {preview.map(fmtSmart).join(" | ")}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
