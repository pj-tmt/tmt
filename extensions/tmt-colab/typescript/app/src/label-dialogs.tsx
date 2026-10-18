import { BrowserAction, BrowserDrawer, BrowserField } from '@tmt/browser-ui/react';
import { useId, useState } from 'react';
import {
  labelNameError,
  relabel,
  withLabel,
  type LabelChange,
  type LabelOutcome,
} from './labels.js';
import { pageTitle } from './page-index.js';
import { text } from './strings.js';
import type { PageSummary } from './transport.js';
import './label-dialogs.css';

const t = text.pageList;

/** What the owner needs to finish a write: Home applies the changes and refreshes the list. */
export interface LabelWriting {
  save(changes: readonly LabelChange[]): Promise<LabelOutcome>;
  failureText(failure: unknown): string;
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
          value={value}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    />
  );
}

/** A label exists only on pages, so a new one is named and given its first pages together. */
export function NewLabelDialog({
  open,
  pages,
  initial,
  writing,
  onClose,
}: {
  open: boolean;
  pages: readonly PageSummary[];
  /** Pages ticked when the dialog opens. */
  initial: readonly string[];
  writing: LabelWriting;
  onClose(): void;
}) {
  return (
    <BrowserDrawer open={open} keepMounted={false} title={t.newLabel} onDismiss={onClose}>
      {/* Unmounted when closed, so every open starts a fresh form. */}
      <NewLabelForm pages={pages} initial={initial} writing={writing} onClose={onClose} />
    </BrowserDrawer>
  );
}
function NewLabelForm({
  pages,
  initial,
  writing,
  onClose,
}: {
  pages: readonly PageSummary[];
  initial: readonly string[];
  writing: LabelWriting;
  onClose(): void;
}) {
  const [name, setName] = useState('');
  const [ticked, setTicked] = useState(() => new Set(initial));
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const nameError = name === '' ? undefined : labelNameError(name);
  async function create() {
    const invalid = labelNameError(name);
    if (invalid) return setError(t.nameErrors[invalid]);
    const changes: LabelChange[] = [];
    for (const page of pages.filter((candidate) => ticked.has(candidate.id))) {
      const change = withLabel(page, name, true);
      if (change === 'capacity') return setError(`${pageTitle(page)}: ${t.pageFull}`);
      changes.push(change);
    }
    setBusy(true);
    setError(undefined);
    const outcome = await writing.save(changes);
    setBusy(false);
    if (outcome.failure === undefined) return onClose();
    setError(
      `${t.progress(outcome.updated, outcome.total)} ${writing.failureText(outcome.failure)}`,
    );
  }
  return (
    <form
      className="label-form"
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
      <fieldset className="label-pages">
        <legend>{t.pagesField}</legend>
        <p className="label-hint">{t.newLabelHint}</p>
        <ul>
          {pages.map((page) => (
            <li key={page.id}>
              <label className="checkbox-label">
                <input
                  type="checkbox"
                  checked={ticked.has(page.id)}
                  onChange={(event) => {
                    const next = new Set(ticked);
                    if (event.target.checked) next.add(page.id);
                    else next.delete(page.id);
                    setTicked(next);
                    setError(undefined);
                  }}
                />
                {pageTitle(page)}
                {page.archived ? ` · ${text.archived}` : ''}
              </label>
            </li>
          ))}
        </ul>
      </fieldset>
      {error && (
        <p role="alert" className="label-error">
          {error}
        </p>
      )}
      <div className="label-actions">
        <BrowserAction type="button" variant="text" label={t.cancel} onActivate={onClose} />
        <BrowserAction
          type="submit"
          variant="primary"
          label={t.create}
          busy={busy}
          disabled={name === '' || nameError !== undefined || ticked.size === 0}
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

/** Rename and delete reach every page that carries the name, archived or not, whatever the list shows. */
export function ManageLabelsDialog({
  open,
  pages,
  labels,
  readPages,
  writing,
  onClose,
}: {
  open: boolean;
  pages: readonly PageSummary[];
  labels: readonly { name: string; count: number }[];
  /** A fresh read of the whole space, so a retry plans only what is left. */
  readPages(): Promise<readonly PageSummary[]>;
  writing: LabelWriting;
  onClose(): void;
}) {
  const [step, setStep] = useState<Step>({ kind: 'list' });
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; failed: boolean }>();
  function close() {
    if (busy) return;
    setStep({ kind: 'list' });
    setMessage(undefined);
    onClose();
  }
  async function run(from: string, to?: string) {
    setBusy(true);
    setMessage(undefined);
    let outcome: LabelOutcome;
    try {
      const changes = relabel(await readPages(), from, to);
      outcome = changes.length ? await writing.save(changes) : { updated: 0, total: 0 };
    } catch (failure) {
      outcome = { updated: 0, total: 0, failure };
    }
    setBusy(false);
    if (outcome.failure === undefined) {
      setStep({ kind: 'list' });
      setMessage({ text: t.done(outcome.total), failed: false });
      return;
    }
    setMessage({
      text: `${t.progress(outcome.updated, outcome.total)} ${writing.failureText(outcome.failure)}`,
      failed: true,
    });
  }
  const carrying = (name: string) => pages.filter((page) => page.projects?.includes(name));
  return (
    <BrowserDrawer open={open} keepMounted={false} title={t.manageLabels} onDismiss={close}>
      {step.kind === 'list' && (
        <div className="label-form">
          {labels.length ? (
            <ul className="label-manage">
              {labels.map(({ name, count }) => (
                <li key={name}>
                  <span className="label-manage-text">
                    <span className="label-manage-name">{name}</span>
                    <span className="label-manage-count">
                      {t.pageCount(count, carrying(name).filter((page) => page.archived).length)}
                    </span>
                  </span>
                  <span className="label-manage-actions">
                    <BrowserAction
                      type="button"
                      variant="text"
                      label={`${t.rename} ${name}`}
                      visibleLabel={t.rename}
                      onActivate={() => {
                        setMessage(undefined);
                        setStep({ kind: 'rename', name, to: name });
                      }}
                    />
                    <BrowserAction
                      type="button"
                      variant="destructive"
                      label={`${t.remove} ${name}`}
                      visibleLabel={t.remove}
                      onActivate={() => {
                        setMessage(undefined);
                        setStep({ kind: 'delete', name });
                      }}
                    />
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p>{t.noLabels}</p>
          )}
        </div>
      )}
      {step.kind === 'rename' && (
        <RenameStep
          from={step.name}
          to={step.to}
          pages={carrying(step.name)}
          busy={busy}
          onChange={(to) => setStep({ ...step, to })}
          onConfirm={() => void run(step.name, step.to)}
          onCancel={() => {
            setMessage(undefined);
            setStep({ kind: 'list' });
          }}
        />
      )}
      {step.kind === 'delete' && (
        <div className="label-form">
          <p>
            {t.removeConfirm(
              step.name,
              carrying(step.name).length,
              carrying(step.name).filter((page) => page.archived).length,
            )}
          </p>
          <div className="label-actions">
            <BrowserAction
              type="button"
              variant="text"
              label={t.cancel}
              disabled={busy}
              onActivate={() => {
                setMessage(undefined);
                setStep({ kind: 'list' });
              }}
            />
            <BrowserAction
              type="button"
              variant="destructive"
              label={message?.failed ? t.retry : t.remove}
              busy={busy}
              onActivate={() => void run(step.name)}
            />
          </div>
        </div>
      )}
      {message && (
        <p
          role={message.failed ? 'alert' : 'status'}
          className="label-error"
          data-tone={message.failed ? 'error' : 'ok'}
        >
          {message.text}
        </p>
      )}
      {step.kind === 'list' && (
        <div className="label-actions label-footer">
          <BrowserAction type="button" variant="primary" label={t.close} onActivate={close} />
        </div>
      )}
    </BrowserDrawer>
  );
}
function RenameStep({
  from,
  to,
  pages,
  busy,
  onChange,
  onConfirm,
  onCancel,
}: {
  from: string;
  to: string;
  pages: readonly PageSummary[];
  busy: boolean;
  onChange(to: string): void;
  onConfirm(): void;
  onCancel(): void;
}) {
  const invalid = to === '' ? undefined : labelNameError(to);
  return (
    <form
      className="label-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy && to !== from && !invalid && to !== '') onConfirm();
      }}
    >
      <NameField
        label={t.renameTo}
        value={to}
        error={invalid ? t.nameErrors[invalid] : undefined}
        onChange={onChange}
      />
      {to !== from && !invalid && to !== '' && (
        <p>
          {t.renameConfirm(from, to, pages.length, pages.filter((page) => page.archived).length)}
        </p>
      )}
      <div className="label-actions">
        <BrowserAction
          type="button"
          variant="text"
          label={t.cancel}
          disabled={busy}
          onActivate={onCancel}
        />
        <BrowserAction
          type="submit"
          variant="primary"
          label={t.rename}
          busy={busy}
          disabled={to === from || to === '' || invalid !== undefined}
          onActivate={() => {}}
        />
      </div>
    </form>
  );
}
