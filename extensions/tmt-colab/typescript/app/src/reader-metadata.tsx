import { Check, Diamond, X } from 'lucide-react';
import {
  BrowserAction,
  BrowserDialog,
  BrowserField,
  BrowserNotice,
  BusyMark,
} from '@tmt/browser-ui/react';
import { createPortal } from 'react-dom';
import { useEffect, useId, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { PageAttribution, creatorName } from './page-attribution.js';
import { QuestionRegistryView } from './question-registry-view.js';
import type { MetadataInput, MetadataOutcome, ReaderMetadata } from './link-metadata.js';
import type { PageView } from './transport.js';
import { text } from './strings.js';
import './metadata-dialogs.css';
const t = text.metadata;
const noStatus = () => undefined;
const noSubscription = () => () => {};
interface Status {
  key: string;
  busy: boolean;
  outcome?: MetadataOutcome;
  error?: string;
}

/** Link controls only prepare requests. Owner admission and identity resolution stay remote. */
export function ReaderMetadataInfo({
  view,
  metadata,
  onOpen,
  focusFallback,
}: {
  view: PageView;
  metadata?: ReaderMetadata;
  onOpen(): void;
  focusFallback(): HTMLElement | null;
}) {
  const [creator, setCreator] = useState<PageView>();
  const observed = useSyncExternalStore(
    metadata?.subscribe ?? noSubscription,
    metadata?.snapshot ?? noStatus,
    noStatus,
  );
  const [error, setError] = useState<{ key: string; message: string }>();
  const status: Status | undefined = error
    ? { key: error.key, busy: false, error: error.message }
    : observed
      ? {
          key: observed.request.action === 'creator.set' ? 'creator' : observed.request.body.id,
          busy: observed.busy,
          outcome: observed.outcome,
        }
      : undefined;
  const alive = useRef(true);
  const inFlight = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const enabled = metadata !== undefined && !view.archived && view.pageType !== 'shell';
  const blocked = status?.busy || status?.outcome?.state === 'unconfirmed';
  async function send(input: MetadataInput, key: string): Promise<MetadataOutcome | undefined> {
    if (!metadata || !enabled || blocked || inFlight.current) return;
    inFlight.current = true;
    setError(undefined);
    try {
      return await metadata.send(input);
    } catch {
      if (alive.current) setError({ key, message: t.requestInvalid });
    } finally {
      inFlight.current = false;
    }
  }
  async function check() {
    if (!metadata || !status?.outcome || status.busy) return;
    await metadata.check(status.outcome.operationId);
  }
  const notice = status && (
    <div data-testid="reader-metadata-status">
      <BrowserNotice
        mark={
          status.busy ? (
            <BusyMark />
          ) : status.outcome?.state === 'applied' ? (
            <Check />
          ) : status.outcome?.state === 'unconfirmed' ? (
            <Diamond fill="currentColor" />
          ) : (
            <X />
          )
        }
        tone={
          status.busy || status.outcome?.state === 'unconfirmed'
            ? 'waiting'
            : status.outcome?.state === 'applied'
              ? 'muted'
              : 'blocked'
        }
        announcement={status.error || status.outcome?.state === 'refused' ? 'alert' : 'status'}
        stateLabel={
          status.busy
            ? t.changing
            : status.outcome?.state === 'applied'
              ? t.applied
              : status.outcome?.state === 'unconfirmed'
                ? t.unconfirmed
                : t.notApplied
        }
        title={
          status.busy
            ? t.changing
            : (status.error ??
              (status.outcome?.state === 'applied'
                ? t.applied
                : status.outcome?.state === 'refused'
                  ? status.outcome.message
                  : status.outcome?.state === 'ended'
                    ? text.readerEnded
                    : status.outcome?.state === 'not-sent'
                      ? t.notSent
                      : t.requestUnconfirmed))
        }
        body={null}
        actions={
          status.outcome?.state === 'unconfirmed' && (
            <BrowserAction
              type="button"
              variant="text"
              label={t.checkResult}
              disabled={status.busy}
              onActivate={() => {
                void check();
              }}
            />
          )
        }
      />
    </div>
  );
  return (
    <>
      <section>
        <h3>{text.aboutAuthor}</h3>
        <PageAttribution
          creator={view.creator}
          creationRecipient={view.creationRecipient}
          originalAuthor={view.originalAuthor}
          publisherAgent={view.publisherAgent}
        />
        {enabled && (
          <BrowserAction
            type="button"
            variant="text"
            label={t.changeCreatorItem}
            disabled={!!blocked}
            onActivate={() => {
              onOpen();
              setCreator(view);
            }}
          />
        )}
        {notice}
      </section>
      {view.questions !== undefined && (
        <section>
          <h3>{t.questions}</h3>
          {!enabled && <p>{view.archived ? t.archived : t.ownerOnly}</p>}
          <QuestionRegistryView
            view={view}
            renderAction={
              enabled
                ? (question) => (
                    <BrowserAction
                      type="button"
                      variant="text"
                      label={question.state === 'handled' ? t.reopen : t.handle}
                      busy={status?.busy && status.key === question.id}
                      disabled={!!blocked}
                      onActivate={() => {
                        const target = view.questions?.find((entry) => entry.id === question.id);
                        if (target)
                          void send(
                            {
                              action:
                                question.state === 'handled'
                                  ? 'question.reopen'
                                  : 'question.handle',
                              body: { id: question.id },
                              target: { ...target },
                            },
                            question.id,
                          );
                      }}
                    />
                  )
                : undefined
            }
          />
        </section>
      )}
      {creator &&
        metadata &&
        createPortal(
          <CreatorRequest
            key={metadata.scope}
            view={creator}
            busy={!!status?.busy}
            blocked={!!blocked}
            notice={status?.key === 'creator' ? notice : undefined}
            focusFallback={focusFallback}
            onDismiss={() => setCreator(undefined)}
            send={(selector) =>
              send(
                {
                  action: 'creator.set',
                  body: { selector },
                  target: {
                    creator: creator.creator ?? null,
                    creationRecipient: creator.creationRecipient ?? null,
                  },
                },
                'creator',
              )
            }
          />,
          document.body,
        )}
    </>
  );
}
function CreatorRequest({
  view,
  busy,
  blocked,
  notice,
  focusFallback,
  onDismiss,
  send,
}: {
  view: PageView;
  busy: boolean;
  blocked: boolean;
  notice: ReactNode;
  focusFallback(): HTMLElement | null;
  onDismiss(): void;
  send(selector: string): Promise<MetadataOutcome | undefined>;
}) {
  const fieldId = useId();
  const [selector, setSelector] = useState('');
  const [submitted, setSubmitted] = useState(false);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const value = selector.trim();
  const valid =
    value.length > 0 && new TextEncoder().encode(value).length <= 256 && !/\p{Cc}/u.test(value);
  async function submit() {
    if (!valid || blocked) return;
    setSubmitted(true);
    const outcome = await send(value);
    if (alive.current && outcome?.state === 'applied') onDismiss();
  }
  return (
    <BrowserDialog
      open
      title={t.changeCreator}
      focusFallback={focusFallback}
      onDismiss={onDismiss}
      actions={
        <>
          <BrowserAction type="button" variant="secondary" label={t.close} onActivate={onDismiss} />
          <BrowserAction
            type="button"
            variant="primary"
            label={t.changeCreator}
            busy={busy}
            disabled={!valid || blocked}
            onActivate={() => {
              void submit();
            }}
          />
        </>
      }
    >
      <div className="metadata-editor">
        <p>{t.currentCreator(creatorName(view) ?? t.noCreator)}</p>
        <p>{t.creatorHint}</p>
        <BrowserField
          controlId={fieldId}
          label={t.creatorSelector}
          renderControl={(field) => (
            <input
              {...field}
              value={selector}
              disabled={busy || blocked}
              onChange={(event) => setSelector(event.target.value)}
            />
          )}
        />
        {submitted && notice}
      </div>
    </BrowserDialog>
  );
}
