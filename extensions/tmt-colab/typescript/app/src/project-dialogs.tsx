import {
  BrowserAction,
  BrowserCheckbox,
  BrowserDialog,
  BrowserDrawer,
  BrowserField,
} from '@tmt/browser-ui/react';
import { useEffect, useId, useRef, useState } from 'react';
import { FileText } from 'lucide-react';
import {
  projectNameError,
  withProject,
  type ProjectChange,
  type ProjectManage,
  type ProjectOutcome,
} from './projects.js';
import { ManagementError } from './management.js';
import { pageTitle } from './page-index.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';
import './project-dialogs.css';

const t = text.pageList;

/** What the owner needs to finish a write: Home applies the changes and refreshes the list. */
export interface ProjectWriting {
  /** Replace the sets of the named pages (a name no project has yet creates it). */
  save(changes: readonly ProjectChange[]): Promise<ProjectOutcome>;
  /** Create, rename or delete a project: one request, no page named. */
  manage(change: ProjectManage): Promise<{ failure?: unknown }>;
  failureText(failure: unknown): string;
}

/** The words for a failed project write, whichever surface asked for it. */
export function projectFailureText(failure: unknown): string {
  const code = failure instanceof ManagementError ? failure.code : 'UNKNOWN';
  return t.failures[code as keyof typeof t.failures] ?? t.failures.UNKNOWN;
}

function NameField({
  label,
  value,
  error,
  onChange,
}: {
  label: string;
  value: string;
  error?: string;
  onChange(value: string): void;
}) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus({ preventScroll: true }), []);
  return (
    <BrowserField
      controlId={`${id}-name`}
      label={label}
      error={error}
      errorId={`${id}-error`}
      invalid={error !== undefined}
      renderControl={(field) => (
        <input
          {...field}
          ref={input}
          placeholder={t.projectNamePlaceholder}
          value={value}
          autoComplete="off"
          required
          spellCheck={false}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    />
  );
}

/** A new project is named and may be given its first pages together; none ticked makes it empty. */
export function NewProjectDialog({
  open,
  pages,
  existing,
  initial,
  writing,
  onClose,
}: {
  open: boolean;
  pages: readonly PageSummary[];
  /** The projects the space already has: a new one needs another name. */
  existing: readonly string[];
  /** Pages ticked when the dialog opens. */
  initial: readonly string[];
  writing: ProjectWriting;
  onClose(): void;
}) {
  // A save in flight vetoes every close route, as in ManageProjectsDialog.
  const saving = useRef(false);
  return (
    <BrowserDrawer
      open={open}
      keepMounted={false}
      className="project-surface"
      title={t.newProject}
      onDismiss={() => {
        if (!saving.current) onClose();
      }}
    >
      {/* Unmounted when closed, so every open starts a fresh form. */}
      <NewProjectForm
        pages={pages}
        existing={existing}
        initial={initial}
        writing={writing}
        saving={saving}
        onClose={onClose}
      />
    </BrowserDrawer>
  );
}
function NewProjectForm({
  pages,
  existing,
  initial,
  writing,
  saving,
  onClose,
}: {
  pages: readonly PageSummary[];
  existing: readonly string[];
  initial: readonly string[];
  writing: ProjectWriting;
  saving: { current: boolean };
  onClose(): void;
}) {
  const [name, setName] = useState('');
  const [ticked, setTicked] = useState(() => new Set(initial));
  const [error, setError] = useState<string>();
  const [busy, setBusyState] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      saving.current = false;
    };
  }, [saving]);
  function setBusy(value: boolean) {
    saving.current = value;
    setBusyState(value);
  }
  const nameError = name === '' ? undefined : projectNameError(name);
  async function create() {
    const invalid = projectNameError(name);
    if (invalid) return setError(t.nameErrors[invalid]);
    if (existing.includes(name)) return setError(t.nameErrors.exists);
    const changes: ProjectChange[] = [];
    for (const page of pages.filter((candidate) => ticked.has(candidate.id))) {
      const change = withProject(page, name, true);
      if (change === 'capacity') return setError(`${pageTitle(page)}: ${t.pageFull}`);
      changes.push(change);
    }
    setBusy(true);
    setError(undefined);
    if (changes.length === 0) {
      const created = await writing.manage({ action: 'create', name });
      // A completion belongs to this opening only: it never closes a later one.
      if (!mounted.current) return;
      setBusy(false);
      if (created.failure === undefined) return onClose();
      return setError(writing.failureText(created.failure));
    }
    const outcome = await writing.save(changes);
    if (!mounted.current) return;
    setBusy(false);
    if (outcome.failure === undefined) return onClose();
    setError(
      `${t.progress(outcome.updated, outcome.total)} ${writing.failureText(outcome.failure)}`,
    );
  }
  return (
    <form
      className="project-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy) void create();
      }}
    >
      <NameField
        label={t.nameField}
        value={name}
        error={nameError ? t.nameErrors[nameError] : undefined}
        onChange={(value) => {
          setName(value);
          setError(undefined);
        }}
      />
      <div className="project-divider" role="separator" />
      <fieldset className="project-pages">
        <legend>{t.pagesField}</legend>
        <p className="project-hint">{t.newProjectHint}</p>
        <ul>
          {pages.map((page) => (
            <li key={page.id}>
              <label className="checkbox-label">
                <BrowserCheckbox
                  checked={ticked.has(page.id)}
                  onChange={(event) => {
                    const next = new Set(ticked);
                    if (event.target.checked) next.add(page.id);
                    else next.delete(page.id);
                    setTicked(next);
                    setError(undefined);
                  }}
                />
                <FileText size={14} aria-hidden="true" />
                <span>
                  {pageTitle(page)}
                  {page.archived ? ` · ${text.archived}` : ''}
                </span>
              </label>
            </li>
          ))}
        </ul>
      </fieldset>
      {error && (
        <p role="alert" className="project-error">
          {error}
        </p>
      )}
      <div className="project-actions">
        <BrowserAction
          type="button"
          variant="secondary"
          label={t.cancel}
          disabled={busy}
          onActivate={onClose}
        />
        <BrowserAction
          type="submit"
          variant="primary"
          label={t.create}
          busy={busy}
          disabled={nameError !== undefined}
          onActivate={() => {}}
        />
      </div>
    </form>
  );
}

type Step =
  | { kind: 'list' }
  | { kind: 'rename'; name: string; to: string }
  | { kind: 'delete'; name: string };

/** Rename and delete reach every page in the project, archived or not, whatever the list shows. */
export function ManageProjectsDialog({
  open,
  pages,
  projects,
  writing,
  onClose,
  onNewProject,
}: {
  open: boolean;
  pages: readonly PageSummary[];
  /** Every project of the space, including those with no pages. */
  projects: readonly { name: string; count: number }[];
  writing: ProjectWriting;
  onClose(): void;
  onNewProject(): void;
}) {
  const [step, setStep] = useState<Step>({ kind: 'list' });
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; failed: boolean }>();
  const manager = useRef<HTMLDivElement>(null);
  const returnAction = useRef<string | undefined>(undefined);
  const removal = useRef<HTMLParagraphElement>(null);
  // The manager is replaced while a confirmation owns modality. Its buttons are new nodes when
  // it returns, so restore by the action's domain name rather than a detached opener element.
  useEffect(() => {
    if (step.kind === 'delete') removal.current?.focus({ preventScroll: true });
    if (step.kind !== 'list' || !returnAction.current) return;
    const target = Array.from(manager.current?.querySelectorAll('button') ?? []).find(
      (button) => button.getAttribute('aria-label') === returnAction.current,
    );
    if (target) {
      target.focus({ preventScroll: true });
      returnAction.current = undefined;
    }
  }, [step, projects]);
  function cancel() {
    if (busy) return;
    setStep({ kind: 'list' });
    setMessage(undefined);
  }
  function close() {
    if (busy) return;
    returnAction.current = undefined;
    cancel();
    onClose();
  }
  async function run(name: string, to?: string) {
    setBusy(true);
    setMessage(undefined);
    const outcome = await writing.manage(
      to === undefined ? { action: 'delete', name } : { action: 'rename', name, to },
    );
    setBusy(false);
    if (outcome.failure === undefined) {
      returnAction.current = to === undefined ? t.newProject : `${t.rename} ${to}`;
      setStep({ kind: 'list' });
      setMessage({ text: t.projectSaved, failed: false });
    } else setMessage({ text: writing.failureText(outcome.failure), failed: true });
  }
  const carrying = (name: string) => pages.filter((page) => page.projects?.includes(name));
  return (
    <>
      <BrowserDrawer
        open={open && step.kind === 'list'}
        keepMounted={false}
        title={t.manageProjects}
        className="project-surface"
        onDismiss={close}
      >
        <div className="project-form" ref={manager}>
          <p className="project-hint">{t.manageProjectsHint}</p>
          {projects.length ? (
            <ul className="project-manage">
              {projects.map(({ name, count }) => {
                const archived = carrying(name).filter((page) => page.archived).length;
                return (
                  <li key={name}>
                    <div className="project-manage-row">
                      <span className="project-manage-text">
                        <span className="project-manage-name">{name}</span>
                        <span className="project-manage-count">
                          {t.activeProjectCount(count - archived, archived)}
                        </span>
                      </span>
                      <span className="project-manage-actions">
                        <BrowserAction
                          type="button"
                          variant="secondary"
                          label={`${t.rename} ${name}`}
                          visibleLabel={t.rename}
                          onActivate={() => {
                            returnAction.current = `${t.rename} ${name}`;
                            setMessage(undefined);
                            setStep({ kind: 'rename', name, to: name });
                          }}
                        />
                        <BrowserAction
                          type="button"
                          variant="text"
                          label={`${t.remove} ${name}`}
                          visibleLabel={t.remove}
                          onActivate={() => {
                            returnAction.current = `${t.remove} ${name}`;
                            setMessage(undefined);
                            setStep({ kind: 'delete', name });
                          }}
                        />
                      </span>
                    </div>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p>{t.noProjects}</p>
          )}
          {message && (
            <p
              role={message.failed ? 'alert' : 'status'}
              className="project-error"
              data-tone={message.failed ? 'error' : 'ok'}
            >
              {message.text}
            </p>
          )}
          <div className="project-actions project-footer">
            <BrowserAction
              type="button"
              variant="primary"
              label={t.newProject}
              visibleLabel={t.newProjectVisible}
              onActivate={() => {
                cancel();
                onNewProject();
              }}
            />
          </div>
        </div>
      </BrowserDrawer>
      <BrowserDialog
        open={open && step.kind !== 'list'}
        title={step.kind === 'delete' ? t.deleteProjectTitle(step.name) : t.renameProjectTitle}
        restoreFocus={false}
        className="project-surface"
        onDismiss={cancel}
        actions={
          step.kind === 'delete' ? (
            <>
              <BrowserAction
                type="button"
                variant="secondary"
                label={t.cancel}
                disabled={busy}
                onActivate={cancel}
              />
              <BrowserAction
                type="button"
                variant="destructive"
                label={message?.failed ? t.retry : t.remove}
                busy={busy}
                onActivate={() => void run(step.name)}
              />
            </>
          ) : undefined
        }
      >
        {step.kind === 'rename' && (
          <RenameStep
            from={step.name}
            to={step.to}
            pages={carrying(step.name)}
            existing={projects.map((project) => project.name)}
            busy={busy}
            onChange={(to) => {
              setMessage(undefined);
              setStep({ ...step, to });
            }}
            onConfirm={() => void run(step.name, step.to)}
            onCancel={cancel}
          />
        )}
        {step.kind === 'delete' && (
          <p ref={removal} tabIndex={-1}>
            {t.removeConfirm(
              step.name,
              carrying(step.name).length,
              carrying(step.name).filter((page) => page.archived).length,
            )}
          </p>
        )}
        {message?.failed && (
          <p role="alert" className="project-error">
            {message.text}
          </p>
        )}
      </BrowserDialog>
    </>
  );
}
function RenameStep({
  from,
  to,
  pages,
  existing,
  busy,
  onChange,
  onConfirm,
  onCancel,
}: {
  from: string;
  to: string;
  pages: readonly PageSummary[];
  existing: readonly string[];
  busy: boolean;
  onChange(to: string): void;
  onConfirm(): void;
  onCancel(): void;
}) {
  const invalid = to === '' ? undefined : projectNameError(to);
  return (
    <form
      className="project-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy && to !== from && !invalid && to !== '') onConfirm();
      }}
    >
      <NameField
        label={t.nameField}
        value={to}
        error={invalid ? t.nameErrors[invalid] : undefined}
        onChange={onChange}
      />
      {to !== from && !invalid && to !== '' && (
        <p>
          {existing.includes(to)
            ? t.mergeConfirm(from, to, pages.length)
            : t.renameConfirm(from, to, pages.length, pages.filter((page) => page.archived).length)}
        </p>
      )}
      <div className="project-actions">
        <BrowserAction
          type="button"
          variant="secondary"
          label={t.cancel}
          disabled={busy}
          onActivate={onCancel}
        />
        <BrowserAction
          type="submit"
          variant="primary"
          label={existing.includes(to) ? t.merge : t.rename}
          busy={busy}
          disabled={to === from || to === '' || invalid !== undefined}
          onActivate={() => {}}
        />
      </div>
    </form>
  );
}
