import { Plus } from 'lucide-react';
import { BrowserAction, BrowserDialog, BrowserField, BrowserMenu } from '@tmt/browser-ui/react';
import {
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
  UiTheme,
} from '@tmt/browser-ui/ui';
import '@tmt/browser-ui/ui.css';
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
  type FormEvent,
  type ReactNode,
} from 'react';
import { SaveNotApplied, SaveRefused, saveMessage, type WorkAction } from './save.js';
import { text } from './strings.js';
import { getTheme, subscribeTheme } from './theme.js';
import type { PageBinding, PageSummary, WorkOutcome } from './transport.js';
import { TASK_STATES, keptByEdit, type Task, type TaskState } from './work.js';
import { waitsForSignoff } from './work-people.js';

type Body = Record<string, unknown>;
export interface WorkRun {
  /** Send one action; resolves true once the page shows it, false when it was refused. */
  run(action: WorkAction, body: Body): Promise<boolean>;
  runResult(
    action: WorkAction,
    body: Body,
    displayError?: boolean,
  ): Promise<{ value?: WorkOutcome; error?: unknown }>;
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
  const runResult = useCallback(
    async (action: WorkAction, body: Body, displayError = true) => {
      if (!binding?.work || flying.current) return { error: new Error('Work is unavailable') };
      flying.current = true;
      setBusy(true);
      // Linking the page a convert made answers the row that offered it: that row stays.
      const answering = action === 'task.link' && current.current?.page === body.pageId;
      setError(undefined);
      setErrorAt(undefined);
      if (!answering) setStranded(undefined);
      try {
        const value = await binding.work(action, body);
        return { value };
      } catch (failure) {
        const page =
          failure instanceof SaveRefused || failure instanceof SaveNotApplied
            ? failure.childPage
            : undefined;
        if (page !== undefined && action === 'task.convert')
          setStranded({ task: String(body.taskId), page });
        if ((page !== undefined && action === 'task.convert') || answering) setErrorAt('stranded');
        if (displayError)
          setError(
            failure instanceof SaveNotApplied && page !== undefined
              ? text.workBand.controls.madeNotLinked
              : saveMessage(failure),
          );
        return { error: failure };
      } finally {
        flying.current = false;
        setBusy(false);
      }
    },
    [binding],
  );
  const run = useCallback(
    async (action: WorkAction, body: Body) => (await runResult(action, body)).value !== undefined,
    [runResult],
  );
  return { run, runResult, busy, error, stranded, errorAt };
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

interface TaskPart {
  id: string;
  task: Task;
  work: WorkRun;
}

/** One task's controls, in the order they read: sign-off, state, assignee, edit and the rest, and
 * the page a convert made but could not link. Each part owns its own draft and shares `WorkRun`,
 * so one action is in flight at a time and a refusal is told where it belongs. */
export function TaskControls({
  id,
  task,
  state,
  work,
  pageHref,
}: TaskPart & {
  state: TaskState;
  pageHref?(pageId: string): string;
}): ReactNode {
  return (
    <div className="work-controls" data-action-group>
      <StateControls id={id} task={task} state={state} work={work} />
      <AssignForm id={id} task={task} work={work} />
      <EditArea id={id} task={task} work={work} />
      <StrandedRow id={id} work={work} pageHref={pageHref} />
    </div>
  );
}

/** Sign-off and send back, the state menu, and the reason a blocked task asks for. */
function StateControls({ id, task, state, work }: TaskPart & { state: TaskState }): ReactNode {
  const { run, busy } = work;
  const [reason, setReason] = useState<string>();
  const follows = task.child !== undefined;
  const shown = reason === undefined ? state : 'blocked';
  const setState = async (next: TaskState, why?: string) => {
    if (await run('task.state', { taskId: id, state: next, ...(why ? { reason: why } : {}) }))
      setReason(undefined);
  };
  return (
    <>
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
          valueLabel={text.workBand.taskState[shown]}
          disabled={busy || follows}
          items={TASK_STATES.map((value) => ({
            key: value,
            label: text.workBand.taskState[value],
            role: 'menuitemradio' as const,
            checked: value === shown,
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
                  variant="secondary"
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
    </>
  );
}

/** Who the task is assigned to: an agent or a person by name, or nobody. */
function AssignForm({ id, task, work }: TaskPart): ReactNode {
  const { run, busy } = work;
  const [kind, setKind] = useState<'agent' | 'person'>(task.assignee?.kind ?? 'agent');
  const [name, setName] = useState('');
  const assign = async (event: FormEvent) => {
    event.preventDefault();
    if (
      name.trim() &&
      (await run('task.assign', { taskId: id, assignee: { kind, name: name.trim() } }))
    )
      setName('');
  };
  return (
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
              variant="secondary"
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
  );
}

/** The title and detail form while a task is edited, otherwise its row of actions. */
function EditArea({ id, task, work }: TaskPart): ReactNode {
  const [editing, setEditing] = useState(false);
  return editing ? (
    <EditForm id={id} task={task} work={work} onDone={() => setEditing(false)} />
  ) : (
    <TaskActions id={id} task={task} work={work} onEdit={() => setEditing(true)} />
  );
}

/** `task.set` replaces the whole editable set, so what the form does not show is sent back as it is. */
function EditForm({ id, task, work, onDone }: TaskPart & { onDone(): void }): ReactNode {
  const { run, busy } = work;
  const [title, setTitle] = useState(task.title);
  const [detail, setDetail] = useState(task.detail ?? '');
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
      onDone();
  };
  return (
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
          onActivate={onDone}
        />
      </div>
    </form>
  );
}

/** Edit, convert to Colab, and remove after a second click. */
function TaskActions({ id, task, work, onEdit }: TaskPart & { onEdit(): void }): ReactNode {
  const { run, busy } = work;
  const [removing, setRemoving] = useState(false);
  const reasonNotConvertible =
    task.child !== undefined
      ? label.convertFollows
      : task.externalId !== undefined
        ? label.convertExternal
        : undefined;
  return (
    <div className="work-control-group">
      <BrowserAction
        type="button"
        variant="secondary"
        label={label.edit}
        disabled={busy}
        onActivate={onEdit}
      />
      <BrowserAction
        type="button"
        variant="secondary"
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
  );
}

/** A convert that made its page but could not link it: link it, open it, and the refusal. */
function StrandedRow({
  id,
  work,
  pageHref,
}: {
  id: string;
  work: WorkRun;
  pageHref?(pageId: string): string;
}): ReactNode {
  const stranded = work.stranded?.task === id ? work.stranded.page : undefined;
  if (stranded === undefined) return null;
  return (
    <div className="work-control-group work-stranded">
      <span className="work-control-note">{label.pageMade}</span>
      <BrowserAction
        type="button"
        variant="primary"
        label={label.linkIt}
        disabled={work.busy}
        onActivate={() => void work.run('task.link', { taskId: id, pageId: stranded })}
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
            variant="secondary"
            label={label.add}
            disabled={work.busy || !title.trim()}
            onActivate={() => undefined}
          />
        }
      />
    </form>
  );
}

/** Attach an existing page through the owner's two existing work actions. */
export function AttachSubpage({
  pages,
  pageId,
  work,
  run,
}: {
  pages?: readonly PageSummary[];
  pageId: string;
  work: import('./work.js').Work;
  run: WorkRun;
}) {
  const [open, setOpen] = useState(false);
  const [selectedPage, setSelectedPage] = useState('');
  const [query, setQuery] = useState('');
  const theme = useSyncExternalStore(subscribeTheme, getTheme);
  const [retained, setRetained] = useState<{ task: string; page: string }>();
  const [failure, setFailure] = useState<string>();
  const [unsettled, setUnsettled] = useState(false);
  const mounted = useRef(false);
  const attaching = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  // Missing summaries leave parentage unknown; never offer a candidate on incomplete data.
  const complete = pages?.every((page) => page.summary !== undefined) ?? false;
  const candidates = complete
    ? (pages ?? []).filter(
        (page) =>
          page.id !== pageId &&
          !page.archived &&
          page.pageType !== 'shell' &&
          !page.summary?.parent &&
          !pages?.some((child) => child.summary?.parent?.page === page.id),
      )
    : [];
  useEffect(() => {
    if (!retained || work.tasks[retained.task]?.child !== selectedPage) return;
    setUnsettled(false);
    setRetained(undefined);
    setSelectedPage('');
    setQuery('');
    setFailure(undefined);
    setOpen(false);
  }, [retained, selectedPage, work]);
  const commit = async (target: string) => {
    if (run.busy || attaching.current || unsettled || !target) return;
    const selected = candidates.find((page) => page.id === target);
    if (!selected && !retained) return;
    attaching.current = true;
    try {
      setFailure(undefined);
      let task = retained?.task;
      if (!task) {
        const added = await run.runResult('task.add', { task: { title: selected!.title } }, false);
        if (!mounted.current) return;
        if (!added.value?.taskId) {
          const unknown = !(
            added.error instanceof SaveRefused || added.error instanceof SaveNotApplied
          );
          setFailure(
            added.error === undefined
              ? label.taskIdUnavailable
              : unknown
                ? label.attachUnknown
                : saveMessage(added.error),
          );
          if (unknown) setUnsettled(true);
          return;
        }
        task = added.value.taskId;
        setRetained({ task, page: target });
      }
      const linked = await run.runResult('task.link', { taskId: task, pageId: target }, false);
      if (!mounted.current) return;
      if (linked.value) {
        setRetained(undefined);
        setSelectedPage('');
        setQuery('');
        setOpen(false);
        return;
      }
      setRetained({ task, page: target });
      const unknown = !(
        linked.error instanceof SaveRefused || linked.error instanceof SaveNotApplied
      );
      setFailure(
        linked.error === undefined
          ? label.linkOutcomeUnavailable
          : unknown
            ? label.attachUnknown
            : saveMessage(linked.error),
      );
      if (unknown) setUnsettled(true);
    } finally {
      attaching.current = false;
    }
  };
  return (
    <>
      <BrowserAction
        type="button"
        variant="ghost"
        label={label.attachSubpage}
        leadingIcon={<Plus className="tmt-ui-icon" />}
        onActivate={() => {
          setUnsettled(false);
          setFailure(undefined);
          setOpen(true);
        }}
      />
      <BrowserDialog
        open={open}
        title={label.attachSubpage}
        onDismiss={() => {
          if (!run.busy) setOpen(false);
        }}
        actions={
          <>
            <BrowserAction
              type="button"
              variant="secondary"
              label={label.cancel}
              disabled={run.busy}
              onActivate={() => setOpen(false)}
            />
            {retained && (
              <BrowserAction
                type="button"
                variant="primary"
                label={label.retryLink}
                disabled={run.busy || unsettled || !selectedPage}
                onActivate={() => void commit(selectedPage)}
              />
            )}
          </>
        }
      >
        <div className="work-attach-fields">
          <UiTheme theme={theme}>
            <Command
              label={label.attachPage}
              shouldFilter={false}
              onKeyDownCapture={(event) => {
                if (
                  event.key !== 'Enter' ||
                  !event.nativeEvent.isTrusted ||
                  event.nativeEvent.isComposing ||
                  event.altKey ||
                  event.ctrlKey ||
                  event.metaKey ||
                  event.shiftKey ||
                  run.busy ||
                  unsettled
                )
                  return;
                const option = event.currentTarget.querySelector<HTMLElement>(
                  '[cmdk-item][data-selected="true"]',
                );
                const selected = candidates.find((page) => page.id === option?.dataset.pageId);
                if (!selected) return;
                event.preventDefault();
                event.stopPropagation();
                setSelectedPage(selected.id);
                setQuery(selected.title);
                void commit(selected.id);
              }}
            >
              <CommandInput
                aria-label={label.attachPage}
                placeholder={text.quickSwitcher.title}
                value={query}
                onValueChange={setQuery}
                disabled={run.busy || unsettled}
                aria-busy={run.busy || undefined}
              />
              <CommandList aria-label={text.quickSwitcher.results}>
                <CommandEmpty>{text.quickSwitcher.noMatch}</CommandEmpty>
                {candidates
                  .filter((page) =>
                    page.title.toLocaleLowerCase().includes(query.toLocaleLowerCase()),
                  )
                  .map((page) => (
                    <CommandItem
                      key={page.id}
                      value={page.id}
                      data-page-id={page.id}
                      disabled={run.busy || unsettled}
                      onClickCapture={(event) => {
                        if (
                          !event.nativeEvent.isTrusted ||
                          event.button !== 0 ||
                          event.altKey ||
                          event.ctrlKey ||
                          event.metaKey ||
                          run.busy ||
                          unsettled
                        )
                          return;
                        setSelectedPage(page.id);
                        setQuery(page.title);
                        void commit(page.id);
                      }}
                    >
                      {page.title}
                    </CommandItem>
                  ))}
              </CommandList>
            </Command>
          </UiTheme>
          {!pages && <p>{label.pageListUnavailable}</p>}
          {retained && <p>{label.keptTask(retained.task)}</p>}
          {failure && <p role="alert">{failure}</p>}
        </div>
      </BrowserDialog>
    </>
  );
}
