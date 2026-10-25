import { BrowserAction, BrowserDialog, BrowserField } from '@tmt/browser-ui/react';
import { useEffect, useId, useRef, useState, type Ref } from 'react';
import { useAgentDirectory } from './agent-directory.js';
import { Listbox } from './components/listbox.js';
import { QuestionRegistryView } from './question-registry-view.js';
import type { MetadataAction } from './save.js';
import type { PageBinding, PageView } from './transport.js';
import { useNotificationModalPresence } from './reply-notification-context.js';
import { text } from './strings.js';
import './metadata-dialogs.css';
const t = text.metadata;

type Kind = 'creator' | 'questions';
type Props = { view: PageView; binding?: PageBinding; onDismiss(): void };

/** Closing replaces the editor lifetime. A late publication still follows the binding, but cannot
 * dismiss or overwrite a newly opened dialog. Metadata never reads or owns the source draft. */
export function MetadataDialogs({ kind, ...props }: Props & { kind: Kind | null }) {
  return kind === null ? null : <MetadataEditor key={kind} kind={kind} {...props} />;
}
function MetadataEditor({ kind, view, binding, onDismiss }: Props & { kind: Kind }) {
  const fieldId = useId();
  const owner = binding?.canMetadata?.() === true && !view.archived;
  const { directory, retry } = useAgentDirectory(
    kind === 'creator' && owner ? binding?.ask : undefined,
  );
  const [selection, setSelection] = useState(
    view.creator?.agent.id ?? view.creationRecipient?.agentId ?? '',
  );
  const [query, setQuery] = useState('');
  const [listOpen, setListOpen] = useState(true);
  const [revision, setRevision] = useState<string>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const [pending, setPending] = useState<string>();
  const shown = useRef(view);
  shown.current = view;
  const busy = useRef(false);
  const alive = useRef(true);
  const report = useNotificationModalPresence();
  useEffect(() => {
    alive.current = true;
    report(true);
    return () => {
      alive.current = false;
      report(false);
    };
  }, [report]);
  useEffect(() => {
    if (!owner || !binding?.metadataRevision) return;
    let active = true;
    setRevision(undefined);
    void binding.metadataRevision(shown.current).then(
      (value) => {
        if (active) setRevision(value);
      },
      (failure: unknown) => {
        if (active) setError(failure instanceof Error ? failure.message : t.unavailable);
      },
    );
    return () => {
      active = false;
    };
  }, [owner, binding, attempt]);
  async function submit(action: MetadataAction, body: Record<string, unknown>, key: string) {
    if (busy.current || !owner || !revision || !binding?.metadata) return;
    busy.current = true;
    setPending(key);
    setError(undefined);
    try {
      await binding.metadata(action, body, revision);
      if (!alive.current) return;
      if (kind === 'creator') onDismiss();
      else setAttempt((value) => value + 1);
    } catch (failure) {
      if (alive.current) setError(failure instanceof Error ? failure.message : t.unavailable);
    } finally {
      busy.current = false;
      if (alive.current) setPending(undefined);
    }
  }
  const agents =
    directory.state === 'ready'
      ? [...new Map(directory.agents.map((agent) => [agent.agent, agent])).values()]
      : [];
  const selected = agents.find((agent) => agent.agent === selection);
  const current = view.creator?.agent.id ?? view.creationRecipient?.agentId;
  return (
    <BrowserDialog
      open
      title={kind === 'creator' ? t.creatorTitle : t.questions}
      onDismiss={() => {
        if (!busy.current) onDismiss();
      }}
      actions={
        <>
          <BrowserAction
            type="button"
            variant="secondary"
            label={kind === 'creator' ? 'Cancel' : 'Close'}
            disabled={!!pending}
            onActivate={onDismiss}
          />
          {kind === 'creator' && owner && (
            <BrowserAction
              type="button"
              variant="primary"
              label={t.setCreator}
              disabled={!selected || selection === current || !revision}
              busy={pending === 'creator'}
              onActivate={() => {
                if (selected)
                  void submit(
                    'creator.set',
                    { agent: { id: selected.agent, name: selected.agentName } },
                    'creator',
                  );
              }}
            />
          )}
        </>
      }
    >
      <div className="metadata-editor">
        {!owner && <p>{t.ownerOnly}</p>}
        {error && (
          <div role="alert">
            <p>{error}</p>
            <BrowserAction
              type="button"
              variant="secondary"
              label={t.retry}
              disabled={!!pending}
              onActivate={() => {
                setError(undefined);
                setAttempt((value) => value + 1);
                retry();
              }}
            />
          </div>
        )}
        {kind === 'creator' ? (
          <>
            <p>{t.creatorHint}</p>
            <p>
              {t.currentCreator(
                view.creator?.agent.name ??
                  (view.creationRecipient
                    ? (view.originalAuthor ?? view.creationRecipient.agentId)
                    : t.noCreator),
              )}
            </p>
            {selected && <p>{t.selectedCreator(selected.agentName)}</p>}
            {owner &&
              (directory.state === 'ready' ? (
                <BrowserField
                  controlId={fieldId}
                  label={t.findAgent}
                  renderControl={(field) => (
                    <Listbox
                      label={t.creator}
                      presentation="results"
                      options={agents
                        .filter((agent) =>
                          agent.agentName.toLowerCase().includes(query.toLowerCase()),
                        )
                        .map((agent) => ({ value: agent.agent, label: agent.agentName }))}
                      value={selection}
                      disabled={!!pending}
                      onChange={setSelection}
                      inputTrigger={{
                        open: listOpen,
                        onOpenChange: setListOpen,
                        render: (props) => (
                          <input
                            {...field}
                            {...props}
                            className={`${field.className} metadata-creator-search`}
                            ref={props.ref as Ref<HTMLInputElement>}
                            placeholder={t.findAgent}
                            value={query}
                            disabled={!!pending}
                            onChange={(event) => {
                              setQuery(event.target.value);
                              setListOpen(true);
                            }}
                          />
                        ),
                      }}
                    />
                  )}
                />
              ) : directory.state === 'loading' && binding?.ask ? (
                <p role="status">{t.loading}</p>
              ) : (
                <div role="status">
                  <p>
                    {t.directoryUnavailable(
                      directory.state === 'failed' ? directory.code : undefined,
                    )}
                  </p>
                  <BrowserAction
                    type="button"
                    variant="secondary"
                    label={t.retry}
                    onActivate={retry}
                  />
                </div>
              ))}
          </>
        ) : (
          <QuestionRegistryView
            view={view}
            renderAction={
              owner
                ? (question) => (
                    <BrowserAction
                      type="button"
                      variant="secondary"
                      label={question.state === 'handled' ? t.reopen : t.handle}
                      disabled={!revision || (!!pending && pending !== question.id)}
                      busy={pending === question.id}
                      onActivate={() =>
                        void submit(
                          question.state === 'handled' ? 'question.reopen' : 'question.handle',
                          { id: question.id },
                          question.id,
                        )
                      }
                    />
                  )
                : undefined
            }
          />
        )}
      </div>
    </BrowserDialog>
  );
}
