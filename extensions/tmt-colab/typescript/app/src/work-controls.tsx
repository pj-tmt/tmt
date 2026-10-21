import { useCallback, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { saveMessage, type WorkAction } from './save.js';
import { text } from './strings.js';
import type { PageBinding } from './transport.js';
import { TASK_STATES, type Task, type TaskState } from './work.js';
import { waitsForSignoff } from './work-people.js';

type Body = Record<string, unknown>;
export interface WorkRun {
  /** Send one action; resolves true once the page shows it, false when it was refused. */
  run(action: WorkAction, body: Body): Promise<boolean>;
  busy: boolean;
  /** The sentence for the last refusal, in the serve's own words where it gave them. */
  error?: string;
}

/** One action at a time: a control is disabled while one is in flight, and nothing is applied
 * here, so the list and detail change only when the page's own stream brings the commit back. */
export function useWorkRun(binding: Pick<PageBinding, 'work'> | undefined): WorkRun {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const flying = useRef(false);
  const run = useCallback(
    async (action: WorkAction, body: Body) => {
      if (!binding?.work || flying.current) return false;
      flying.current = true;
      setBusy(true);
      setError(undefined);
      try {
        await binding.work(action, body);
        return true;
      } catch (failure) {
        setError(saveMessage(failure));
        return false;
      } finally {
        flying.current = false;
        setBusy(false);
      }
    },
    [binding],
  );
  return { run, busy, error };
}

const label = text.workBand.controls;

/** What a task keeps when only its title and detail are edited: `task.set` replaces the whole
 * editable set, so the rest is sent back as it is. */
function kept(task: Task): Body {
  return {
    ...(task.acceptance ? { acceptance: task.acceptance } : {}),
    ...(task.needsSignoff ? { needsSignoff: true } : {}),
    ...(task.externalId !== undefined ? { externalId: task.externalId } : {}),
  };
}

export function TaskControls({
  id,
  task,
  state,
  work,
}: {
  id: string;
  task: Task;
  state: TaskState;
  work: WorkRun;
}): ReactNode {
  const { run, busy } = work;
  const [reason, setReason] = useState<string>();
  const [kind, setKind] = useState<'agent' | 'person'>(task.assignee?.kind ?? 'agent');
  const [name, setName] = useState('');
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(task.title);
  const [detail, setDetail] = useState(task.detail ?? '');
  const [removing, setRemoving] = useState(false);
  const follows = task.child !== undefined;

  const setState = async (next: TaskState, why?: string) => {
    if (await run('task.state', { taskId: id, state: next, ...(why ? { reason: why } : {}) }))
      setReason(undefined);
  };
  const assign = async (event: FormEvent) => {
    event.preventDefault();
    if (
      name.trim() &&
      (await run('task.assign', { taskId: id, assignee: { kind, name: name.trim() } }))
    )
      setName('');
  };
  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (
      await run('task.set', {
        taskId: id,
        title: title.trim(),
        ...(detail.trim() ? { detail } : {}),
        ...kept(task),
      })
    )
      setEditing(false);
  };

  return (
    <div className="work-controls">
      {waitsForSignoff(task, state) && (
        <p className="work-control-row">
          <button type="button" disabled={busy} onClick={() => setState('done')}>
            {label.signOff}
          </button>
          <button type="button" disabled={busy} onClick={() => setState('in-progress')}>
            {label.sendBack}
          </button>
        </p>
      )}
      <p className="work-control-row">
        <label>
          {label.state}
          <select
            value={reason === undefined ? state : 'blocked'}
            disabled={busy || follows}
            onChange={(event) => {
              const next = event.target.value as TaskState;
              if (next === 'blocked') setReason(task.reason ?? '');
              else void setState(next);
            }}
          >
            {TASK_STATES.map((value) => (
              <option key={value} value={value}>
                {text.workBand.taskState[value]}
              </option>
            ))}
          </select>
        </label>
        {follows && <span className="work-control-note">{label.followsPage}</span>}
      </p>
      {reason !== undefined && (
        <form
          className="work-control-row"
          onSubmit={(event) => {
            event.preventDefault();
            if (reason.trim()) void setState('blocked', reason.trim());
          }}
        >
          <label>
            {label.reason}
            <input
              value={reason}
              maxLength={256}
              onChange={(event) => setReason(event.target.value)}
            />
          </label>
          <button type="submit" disabled={busy || !reason.trim()}>
            {label.block}
          </button>
          <button type="button" disabled={busy} onClick={() => setReason(undefined)}>
            {label.cancel}
          </button>
        </form>
      )}
      <form className="work-control-row" onSubmit={assign}>
        <label>
          {label.assignTo}
          <select
            value={kind}
            disabled={busy}
            onChange={(event) => setKind(event.target.value as 'agent' | 'person')}
          >
            <option value="agent">{label.agent}</option>
            <option value="person">{label.person}</option>
          </select>
        </label>
        <input
          aria-label={label.assigneeName}
          value={name}
          maxLength={128}
          disabled={busy}
          onChange={(event) => setName(event.target.value)}
        />
        <button type="submit" disabled={busy || !name.trim()}>
          {label.assign}
        </button>
        {task.assignee && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void run('task.assign', { taskId: id })}
          >
            {label.unassign}
          </button>
        )}
      </form>
      {editing ? (
        <form className="work-control-edit" onSubmit={save}>
          <label>
            {label.title}
            <input
              value={title}
              maxLength={120}
              onChange={(event) => setTitle(event.target.value)}
            />
          </label>
          <label>
            {label.detail}
            <textarea rows={4} value={detail} onChange={(event) => setDetail(event.target.value)} />
          </label>
          <p className="work-control-row">
            <button type="submit" disabled={busy || !title.trim()}>
              {label.save}
            </button>
            <button type="button" disabled={busy} onClick={() => setEditing(false)}>
              {label.cancel}
            </button>
          </p>
        </form>
      ) : (
        <p className="work-control-row">
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setTitle(task.title);
              setDetail(task.detail ?? '');
              setEditing(true);
            }}
          >
            {label.edit}
          </button>
          {removing ? (
            <>
              <button
                type="button"
                disabled={busy}
                onClick={() => void run('task.remove', { taskId: id })}
              >
                {label.confirmRemove(id)}
              </button>
              <button type="button" disabled={busy} onClick={() => setRemoving(false)}>
                {label.cancel}
              </button>
            </>
          ) : (
            <button type="button" disabled={busy} onClick={() => setRemoving(true)}>
              {label.remove}
            </button>
          )}
        </p>
      )}
    </div>
  );
}

/** A new task, by title (the serve names it), and the last refusal of any control above. */
export function AddTask({ work }: { work: WorkRun }): ReactNode {
  const [title, setTitle] = useState('');
  return (
    <form
      className="work-control-row work-add"
      onSubmit={async (event) => {
        event.preventDefault();
        if (title.trim() && (await work.run('task.add', { task: { title: title.trim() } })))
          setTitle('');
      }}
    >
      <label>
        {label.newTask}
        <input
          value={title}
          maxLength={120}
          disabled={work.busy}
          onChange={(event) => setTitle(event.target.value)}
        />
      </label>
      <button type="submit" disabled={work.busy || !title.trim()}>
        {label.add}
      </button>
      {work.error && (
        <span className="work-control-error" role="alert">
          {work.error}
        </span>
      )}
    </form>
  );
}
