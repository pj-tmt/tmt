import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { Circle, Info, LoaderCircle, X } from 'lucide-react';
import type { AttachmentBinding } from './attachment-service.js';
import { FilesList } from './files-panel.js';
import { BrowserDrawer } from '@tmt/browser-ui/react';
import { ColabHeader } from './colab-header.js';
import { PageAttribution } from './page-attribution.js';
import { ReaderSource, type ReaderEdit } from './reader-source.js';
import { NoticeCard } from './notice-card.js';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { mountRenderer, type RenderState } from './renderer.js';
import { text } from './strings.js';
import type { PageView } from './transport.js';
import './reader-app.css';

export type ReaderState =
  | { kind: 'opening' }
  | { kind: 'ready'; view: PageView }
  | { kind: 'ended' }
  | { kind: 'invalid' }
  | { kind: 'older' }
  | { kind: 'failed' };

/** The shared page: no Ask, export or share controls exist in this entry. It is read-only, except
 * that a link whose role is editor shows the owner's Source drawer and can save page text. */
export function ReaderApp({
  state,
  attachments,
  notice,
  canEdit = false,
  role,
  accessNote,
  edit,
}: {
  state: ReaderState;
  /** Authorized reads of the page's files; absent until the session is open. */
  attachments?: AttachmentBinding;
  notice?: ReactNode;
  /** The link says it may edit. The serve decides; this only changes what the page offers. */
  canEdit?: boolean;
  role?: 'viewer' | 'commenter' | 'editor';
  accessNote?: string;
  /** Saves page text; absent until the session is open. */
  edit?: ReaderEdit;
}) {
  const [files, setFiles] = useState(false);
  const [sourceOpen, setSourceOpen] = useState(false);
  const [writerBlocked, setWriterBlocked] = useState(false);
  const mode = writerBlocked
    ? text.readerOnly
    : role === 'commenter'
      ? 'Can comment'
      : canEdit
        ? text.readerCanEdit
        : text.readerOnly;
  const descriptors = state.kind === 'ready' ? (state.view.attachments ?? []) : [];
  const source = state.kind === 'ready' ? state.view.source : null;
  const title = state.kind === 'ready' ? state.view.title : '';
  const [render, setRender] = useState<RenderState | 'loading'>('loading');
  const host = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLElement>(null);
  const renderer = useRef<Awaited<ReturnType<typeof mountRenderer>> | null>(null);
  const imageInput = useRef({ binding: attachments, descriptors });
  imageInput.current = { binding: attachments, descriptors };
  const imageScope = attachments?.scope?.();
  const imageDescriptors = JSON.stringify(descriptors);
  useEffect(() => {
    renderer.current?.images(imageInput.current);
  }, [attachments, imageDescriptors, imageScope]);
  useEffect(() => {
    if (source === null) return;
    const controller = new AbortController();
    setRender('loading');
    void mountRenderer(host.current!, source, {
      signal: controller.signal,
      onState: setRender,
      viewportInset: () =>
        (bar.current?.offsetHeight ?? 0) + (bar.current?.getBoundingClientRect().top ?? 0),
    })
      .then((handle) => {
        if (controller.signal.aborted) handle.destroy();
        else {
          renderer.current = handle;
          handle.images(imageInput.current);
        }
      })
      .catch(() => {
        if (!controller.signal.aborted) setRender('failed');
      });
    return () => {
      controller.abort();
      renderer.current = null;
    };
  }, [source]);
  return (
    <>
      <ColabHeader
        headerRef={bar}
        caption={
          state.kind === 'ready' && state.view.originalAuthor !== undefined
            ? text.byAuthor(state.view.originalAuthor)
            : undefined
        }
        title={
          state.kind === 'ready'
            ? title || text.unknownPageTitle
            : state.kind === 'opening'
              ? 'Opening page'
              : state.kind === 'ended'
                ? text.readerEnded
                : text.error
        }
        actions={
          <>
            {state.kind === 'ready' && (
              <>
                <span className="chip">{mode}</span>
                <span
                  className={`status ${render === 'ready' ? 'live' : render === 'loading' ? 'waiting' : 'blocked'}`}
                  role="status"
                >
                  {render === 'ready' ? (
                    <Circle className="status-dot" fill="currentColor" aria-hidden />
                  ) : render === 'loading' ? (
                    <LoaderCircle aria-hidden />
                  ) : (
                    <X aria-hidden />
                  )}{' '}
                  {render === 'ready'
                    ? text.readerLive
                    : render === 'loading'
                      ? text.readerLoading
                      : text.readerStopped}
                </span>
              </>
            )}
            {state.kind === 'ready' && edit && (
              <button
                className={ui.action}
                data-variant="text"
                data-testid="source-toggle"
                aria-expanded={sourceOpen}
                onClick={(event) => {
                  if (event.isTrusted) setSourceOpen((open) => !open);
                }}
              >
                {text.source}
              </button>
            )}
            {descriptors.length > 0 && (
              <button
                className={ui.action}
                data-variant="text"
                data-testid="files-toggle"
                aria-expanded={files}
                onClick={(event) => {
                  if (event.isTrusted) setFiles((open) => !open);
                }}
              >
                {text.files} {descriptors.length}
              </button>
            )}
            <details className="reader-information">
              <summary className={ui.action} data-variant="text" aria-label={text.readerInfo}>
                <Info aria-hidden />
              </summary>
              <div className="reader-information-panel">
                {state.kind === 'ready' && (
                  <PageAttribution
                    originalAuthor={state.view.originalAuthor}
                    publisherAgent={state.view.publisherAgent}
                  />
                )}
                {(accessNote || role !== 'commenter') && (
                  <p>{accessNote ?? (canEdit ? text.readerNoteEditor : text.readerNote)}</p>
                )}
                <p>{text.warning}</p>
              </div>
            </details>
          </>
        }
      />
      <main>
        {notice}
        {state.kind === 'ready' ? (
          <section className="page">
            <div className="workspace">
              <div className="canvas">
                <div className="frame-host" ref={host} />
                {(render === 'navigation' || render === 'failed') && (
                  <NoticeCard state="blocked" eyebrow={mode} title={text.blocked}>
                    <p>{render === 'navigation' ? text.navigation : text.failed}</p>
                    <p>{text.limit}</p>
                  </NoticeCard>
                )}
              </div>
            </div>
          </section>
        ) : (
          <NoticeCard
            state={
              state.kind === 'opening' ? 'opening' : state.kind === 'ended' ? 'ended' : 'blocked'
            }
            eyebrow={mode}
            title={
              state.kind === 'opening'
                ? text.readerOpening
                : state.kind === 'ended'
                  ? text.readerEnded
                  : state.kind === 'older'
                    ? text.readerOlderTitle
                    : text.error
            }
          >
            {state.kind === 'ended' && <p>{text.readerEndedNote}</p>}
            {state.kind === 'invalid' && <p>{text.readerInvalid}</p>}
            {state.kind === 'older' && <p>{text.readerOlder}</p>}
            {state.kind === 'failed' && <p>{text.readerFailed}</p>}
          </NoticeCard>
        )}
      </main>
      {state.kind === 'ready' && edit && (
        <ReaderSource
          source={state.view.source}
          edit={edit}
          open={sourceOpen}
          onDismiss={() => setSourceOpen(false)}
          writerBlocked={writerBlocked}
          onWriterInUse={() => setWriterBlocked(true)}
        />
      )}
      {state.kind === 'ready' && (
        <BrowserDrawer
          focusFallback={() => document.querySelector<HTMLButtonElement>('.tmt-ui-header button')}
          open={files && descriptors.length > 0}
          title={text.files}
          panel="files"
          onDismiss={() => setFiles(false)}
        >
          <FilesList descriptors={descriptors} binding={attachments} />
        </BrowserDrawer>
      )}
    </>
  );
}
