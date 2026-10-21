import { BrowserAction, BrowserField, BrowserMenu } from '@tmt/browser-ui/react';
import { useCallback, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { SaveNotApplied, SaveRefused, saveMessage, type WorkAction } from './save.js';
import { text } from './strings.js';
import type { PageBinding } from './transport.js';
import { TASK_STATES, keptByEdit, type Task, type TaskState } from './work.js';
import { waitsForSignoff } from './work-people.js';

type Body = Record<string, unknown>;
export interface WorkRun {
  /** Send one action; resolves true once the page shows it, false when it was refused. */
  run(action: WorkAction, body: Body): Promise<boolean>;
  busy: boolean;
  /** The sentence for the last refusal, in the serve's own words where it gave them. */
  error?: string;
  /** A convert that made its page but could not link it: the task and the page that exists. */
  stranded?: { task: string; page: string };
  /** Where the refusal belongs: `stranded` is the made-but-not-linked row, otherwise the New task
   * field's error slot. A refusal is told under the control that was refused. */
  errorAt?: 'stranded';
}

/** One action at a time: a control is disabled while one is in flight, and nothing is applied
 * here, so the list and detail change only when the page's own stream brings the commit back. */
export function useWorkRun(binding: Pick<PageBinding, 'work'> | undefined): WorkRun {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [stranded, setStranded] = useState<WorkRun['stranded']>();
  const [errorAt, setErrorAt] = useState<WorkRun['errorAt']>();
  const current = useRef<WorkRun['stranded']>(undefined);
  current.current = stranded;
  const flying = useRef(false);
  const run = useCallback(
    async (action: WorkAction, body: Body) => {
      if (!binding?.work || flying.current) return false;
      flying.current = true;
      setBusy(true);
      // Linking the page a convert made answers the row that offered it: that row stays.
      const answering = action === 'task.link' && current.current?.page === body.pageId;
      setError(undefined);
      setErrorAt(undefined);
      if (!answering) setStranded(undefined);
      try {
        await binding.work(action, body);
        return true;
      } catch (failure) {
        const page =
          failure instanceof SaveRefused || failure instanceof SaveNotApplied
            ? failure.childPage
            : undefined;
        if (page !== undefined && action === 'task.convert')
          setStranded({ task: String(body.taskId), page });
        if ((page !== undefined && action === 'task.convert') || answering) setErrorAt('stranded');
        setError(
          failure instanceof SaveNotApplied && page !== undefined
            ? text.workBand.controls.madeNotLinked
            : saveMessage(failure),
        );
        return false;
      } finally {
        flying.current = false;
        setBusy(false);
      }
    },
    [binding],
  );
  return { run, busy, error, stranded, errorAt };
}

const label = text.workBand.controls;

function TextField({
  label: name,
  value,
  onChange,
  maxLength,
  disabled,
  error,
  multiline,
  trailing,
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  maxLength: number;
  disabled?: boolean;
  error?: string;
  multiline?: boolean;
  /** Actions on the field's own row, so the control and its action share one height and the
   * refusal sits under both. */
  trailing?: ReactNode;
}) {
  const id = useId();
  return (
    <BrowserField
      controlId={`${id}-control`}
      label={name}
      error={error === undefined ? undefined : <span role="alert">{error}</span>}
      errorId={`${id}-error`}
      invalid={error !== undefined}
      renderControl={(field) =>
        multiline ? (
          <textarea
            {...field}
            rows={4}
            value={value}
            disabled={disabled}
            onChange={(event) => onChange(event.target.value)}
          />
        ) : (
          <span className="work-field-row">
            <input
              {...field}
              value={value}
              maxLength={maxLength}
              disabled={disabled}
              autoComplete="off"
              onChange={(event) => onChange(event.target.value)}
            />
            {trailing}
          </span>
        )
      }
    />
  );
}

export function TaskControls({
  id,
  task,
  state,
  work,
  pageHref,
}: {
  id: string;
  task: Task;
  state: TaskState;
  work: WorkRun;
  pageHref?(pageId: string): string;
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
  const reasonNotConvertible = follows
    ? label.convertFollows
    : task.externalId !== undefined
      ? label.convertExternal
      : undefined;
  const stranded = work.stranded?.task === id ? work.stranded.page : undefined;

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
        ...keptByEdit(task),
      })
    )
      setEditing(false);
  };

  return (
    <div className="work-controls">
      {waitsForSignoff(task, state) && (
        <div className="work-control-group">
          <BrowserAction
            type="button"
            variant="primary"
            label={label.signOff}
            disabled={busy}
            onActivate={() => void setState('done')}
          />
          <BrowserAction
            type="button"
            variant="text"
            label={label.sendBack}
            disabled={busy}
            onActivate={() => void setState('in-progress')}
          />
        </div>
      )}
      <div className="work-control-group">
        <BrowserMenu
          label={label.state}
          valueLabel={text.workBand.taskState[reason === undefined ? state : 'blocked']}
          disabled={busy || follows}
          items={TASK_STATES.map((value) => ({
            key: value,
            label: text.workBand.taskState[value],
            role: 'menuitemradio' as const,
            checked: value === (reason === undefined ? state : 'blocked'),
          }))}
          onSelect={(next) => {
            if (next === 'blocked') setReason(task.reason ?? '');
            else void setState(next);
          }}
        />
        {follows && <span className="work-control-note">{label.followsPage}</span>}
      </div>
      {reason !== undefined && (
        <form
          className="work-control-group"
          onSubmit={(event) => {
            event.preventDefault();
            if (reason.trim()) void setState('blocked', reason.trim());
          }}
        >
          <TextField
            label={label.reason}
            value={reason}
            maxLength={256}
            onChange={setReason}
            trailing={
              <>
                <BrowserAction
                  type="submit"
                  variant="primary"
                  label={label.block}
                  disabled={busy || !reason.trim()}
                  onActivate={() => undefined}
                />
                <BrowserAction
                  type="button"
                  variant="text"
                  label={label.cancel}
                  disabled={busy}
                  onActivate={() => setReason(undefined)}
                />
              </>
            }
          />
        </form>
      )}
      <form className="work-control-group" onSubmit={assign}>
        <BrowserMenu
          label={label.assignTo}
          valueLabel={kind === 'agent' ? label.agent : label.person}
          disabled={busy}
          items={(['agent', 'person'] as const).map((value) => ({
            key: value,
            label: value === 'agent' ? label.agent : label.person,
            role: 'menuitemradio' as const,
            checked: value === kind,
          }))}
          onSelect={(key) => setKind(key === 'person' ? 'person' : 'agent')}
        />
        <TextField
          label={label.assigneeName}
          value={name}
          maxLength={128}
          disabled={busy}
          onChange={setName}
          trailing={
            <>
              <BrowserAction
                type="submit"
                variant="primary"
                label={label.assign}
                disabled={busy || !name.trim()}
                onActivate={() => undefined}
              />
              {task.assignee && (
                <BrowserAction
                  type="button"
                  variant="text"
                  label={label.unassign}
                  disabled={busy}
                  onActivate={() => void run('task.assign', { taskId: id })}
                />
              )}
            </>
          }
        />
      </form>
      {editing ? (
        <form className="work-control-stack" onSubmit={save}>
          <TextField label={label.title} value={title} maxLength={120} onChange={setTitle} />
          <TextField
            label={label.detail}
            value={detail}
            maxLength={4096}
            multiline
            onChange={setDetail}
          />
          <div className="work-control-group">
            <BrowserAction
              type="submit"
              variant="primary"
              label={label.save}
              disabled={busy || !title.trim()}
              onActivate={() => undefined}
            />
            <BrowserAction
              type="button"
              variant="text"
              label={label.cancel}
              disabled={busy}
              onActivate={() => setEditing(false)}
            />
          </div>
        </form>
      ) : (
        <div className="work-control-group">
          <BrowserAction
            type="button"
            variant="primary"
            label={label.edit}
            disabled={busy}
            onActivate={() => {
              setTitle(task.title);
              setDetail(task.detail ?? '');
              setEditing(true);
            }}
          />
          <BrowserAction
            type="button"
            variant="primary"
            label={label.convert}
            disabled={busy || reasonNotConvertible !== undefined}
            disabledReason={reasonNotConvertible}
            disabledReasonId={`${id}-convert-reason`}
            onActivate={() => void run('task.convert', { taskId: id })}
          />
          {removing ? (
            <>
              <BrowserAction
                type="button"
                variant="destructive"
                label={label.confirmRemove(id)}
                disabled={busy}
                onActivate={() => void run('task.remove', { taskId: id })}
              />
              <BrowserAction
                type="button"
                variant="text"
                label={label.cancel}
                disabled={busy}
                onActivate={() => setRemoving(false)}
              />
            </>
          ) : (
            <BrowserAction
              type="button"
              variant="destructive"
              label={label.remove}
              disabled={busy}
              onActivate={() => setRemoving(true)}
            />
          )}
        </div>
      )}
      {stranded !== undefined && (
        <div className="work-control-group work-stranded">
          <span className="work-control-note">{label.pageMade}</span>
          <BrowserAction
            type="button"
            variant="primary"
            label={label.linkIt}
            disabled={busy}
            onActivate={() => void run('task.link', { taskId: id, pageId: stranded })}
          />
          {pageHref && (
            <a className="tmt-ui-action" data-variant="text" href={pageHref(stranded)}>
              {label.openIt}
            </a>
          )}
          {work.error !== undefined && work.errorAt === 'stranded' && (
            <div className="tmt-ui-field-error work-stranded-error" role="alert">
              {work.error}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/** A new task, by title (the serve names it). The last refusal of any control is told here, in
 * this field's error slot, so it is in one place at every width. */
export function AddTask({ work }: { work: WorkRun }): ReactNode {
  const [title, setTitle] = useState('');
  return (
    <form
      className="work-control-group work-add"
      onSubmit={async (event) => {
        event.preventDefault();
        if (title.trim() && (await work.run('task.add', { task: { title: title.trim() } })))
          setTitle('');
      }}
    >
      <TextField
        label={label.newTask}
        value={title}
        maxLength={120}
        disabled={work.busy}
        error={work.errorAt === 'stranded' ? undefined : work.error}
        onChange={setTitle}
        trailing={
          <BrowserAction
            type="submit"
            variant="primary"
            label={label.add}
            disabled={work.busy || !title.trim()}
            onActivate={() => undefined}
          />
        }
      />
    </form>
  );
}
