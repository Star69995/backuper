import { useEffect, useId, useState } from "react";
import type { Task } from "../types";
import { Button, Field, Modal, TextInput } from "./ui";

/** Name shown for the tasks that aren't in any group. */
export const GENERAL_GROUP = "כללי";

const collator = new Intl.Collator("he", { numeric: true, sensitivity: "base" });

/** The task's group, trimmed ("" = general). */
export const groupOf = (t: Pick<Task, "group">) => (t.group ?? "").trim();

/** Names of the groups in use, sorted. */
export const groupNames = (tasks: Task[]) =>
  [...new Set(tasks.map(groupOf).filter(Boolean))].sort((a, b) => collator.compare(a, b));

export const sameGroup = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/** Free text with the existing groups as suggestions; empty = general list. */
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
  const listId = useId();
  return (
    <>
      <TextInput
        className={className}
        value={value}
        list={listId}
        placeholder={`${GENERAL_GROUP} (ללא קבוצה)`}
        onChange={(e) => onChange(e.target.value)}
      />
      <datalist id={listId}>
        {groups.map((g) => (
          <option key={g} value={g} />
        ))}
      </datalist>
    </>
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
