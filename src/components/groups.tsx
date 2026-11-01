import { Check, ChevronDown, List, Plus } from "lucide-react";
import { useEffect, useState } from "react";
import type { Task } from "../types";
import { Button, cx, Field, IconButton, inputCls, Menu, type MenuItem, Modal, TextInput } from "./ui";

/** Name shown for the tasks that aren't in any group. */
export const GENERAL_GROUP = "כללי";

const collator = new Intl.Collator("he", { numeric: true, sensitivity: "base" });

/** The task's group, trimmed ("" = general). */
export const groupOf = (t: Pick<Task, "group">) => (t.group ?? "").trim();

/** Names of the groups in use, sorted. */
export const groupNames = (tasks: Task[]) =>
  [...new Set(tasks.map(groupOf).filter(Boolean))].sort((a, b) => collator.compare(a, b));

export const sameGroup = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * Picks the general list or one of the existing groups from a dropdown menu; "קבוצה חדשה..." swaps the
 * menu for a text field (with a button back to the list). With no groups yet it is just the text field.
 * Empty value = general list.
 */
export function GroupInput({
  value,
  onChange,
  groups,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  groups: string[];
  className?: string;
}) {
  const [adding, setAdding] = useState(false);
  const name = value.trim();
  const existing = groups.find((g) => sameGroup(g, name));
  if (adding || groups.length === 0) {
    return (
      <div className={cx("flex min-w-0 items-center gap-1.5", className)}>
        <TextInput
          className="min-w-0 flex-1"
          autoFocus={adding}
          value={value}
          placeholder="שם הקבוצה החדשה"
          onChange={(e) => onChange(e.target.value)}
        />
        {groups.length > 0 && (
          <IconButton label="בחירה מהקבוצות הקיימות" onClick={() => setAdding(false)}>
            <List size={16} />
          </IconButton>
        )}
      </div>
    );
  }
  const current = existing ?? name;
  const choice = (label: string, v: string): MenuItem => ({
    label,
    icon: current === v ? <Check size={14} /> : undefined,
    onClick: () => onChange(v),
  });
  return (
    <div className={cx("min-w-0", className)}>
      <Menu
        fill
        items={[
          choice(`${GENERAL_GROUP} (ללא קבוצה)`, ""),
          ...groups.map((g) => choice(g, g)),
          // A name typed earlier (or taken from the list file) that no task uses yet.
          ...(name && !existing ? [choice(`${name} (חדשה)`, name)] : []),
          "separator",
          {
            label: "קבוצה חדשה...",
            icon: <Plus size={14} />,
            onClick: () => {
              setAdding(true);
              onChange("");
            },
          },
        ]}
        trigger={(open) => (
          <button
            type="button"
            aria-haspopup="menu"
            onClick={open}
            className={cx(inputCls, "flex w-full cursor-pointer items-center justify-between gap-2 text-start")}
          >
            <span className="truncate">{current || `${GENERAL_GROUP} (ללא קבוצה)`}</span>
            <ChevronDown size={14} className="shrink-0 text-muted" />
          </button>
        )}
      />
    </div>
  );
}

/** Asks for a group name (new group / rename). */
export function GroupNameDialog({
  title,
  initial = "",
  confirmLabel,
  taken,
  onClose,
  onSubmit,
}: {
  title: string;
  initial?: string;
  confirmLabel: string;
  /** Names already in use (other than `initial`). */
  taken: string[];
  onClose: () => void;
  onSubmit: (name: string) => void;
}) {
  const [name, setName] = useState(initial);
  const [touched, setTouched] = useState(false);
  useEffect(() => setName(initial), [initial]);
  const trimmed = name.trim();
  const error = !trimmed
    ? "יש לתת שם לקבוצה"
    : sameGroup(trimmed, GENERAL_GROUP)
      ? `השם "${GENERAL_GROUP}" שמור למשימות שאינן בקבוצה`
      : taken.some((g) => sameGroup(g, trimmed) && !sameGroup(g, initial))
        ? "כבר יש קבוצה בשם הזה"
        : null;
  const submit = () => {
    setTouched(true);
    if (!error) onSubmit(trimmed);
  };
  return (
    <Modal
      title={title}
      size="sm"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>ביטול</Button>
          <Button variant="primary" onClick={submit}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <Field label="שם הקבוצה" error={touched ? error : null}>
          <TextInput autoFocus value={name} placeholder="למשל: מחשב העבודה" onChange={(e) => setName(e.target.value)} />
        </Field>
      </form>
    </Modal>
  );
}
