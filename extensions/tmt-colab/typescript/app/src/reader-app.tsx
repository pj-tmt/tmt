import { useEffect, useRef, useState } from 'react';
import { mountRenderer, type RenderState } from './renderer.js';
import { text } from './strings.js';
import type { PageView } from './transport.js';

export type ReaderState =
  | { kind: 'opening' }
  | { kind: 'ready'; view: PageView }
  | { kind: 'ended' }
  | { kind: 'invalid' }
  | { kind: 'failed' };

/** The read-only page: no editor, Ask, export or share controls exist in this entry. */
export function ReaderApp({ state }: { state: ReaderState }) {
  const source = state.kind === 'ready' ? state.view.source : null;
  const title = state.kind === 'ready' ? state.view.title : '';
  const [render, setRender] = useState<RenderState | 'loading'>('loading');
  const host = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (source === null) return;
    const controller = new AbortController();
    setRender('loading');
    void mountRenderer(host.current!, source, {
      signal: controller.signal,
      onState: setRender,
    }).catch(() => {
      if (!controller.signal.aborted) setRender('failed');
    });
    return () => controller.abort();
  }, [source]);
  return (
    <>
      <header className="masthead">
        <span className="brand">
          {text.product}
          <span>tmt</span>
        </span>
        <span className="local">{text.readerOnly}</span>
      </header>
      <main>
        {state.kind === 'ready' ? (
          <section className="page">
            <div className="page-bar">
              <h1>{title}</h1>
              <span className="chip">{text.readerOnly}</span>
              <span className={`status ${render === 'ready' ? 'live' : ''}`}>
                <span aria-hidden>
                  {render === 'ready' ? '●' : render === 'loading' ? '○' : '✗'}
                </span>{' '}
                {render === 'ready'
                  ? text.loaded
                  : render === 'loading'
                    ? text.loading
                    : text.blocked}
              </span>
            </div>
            <div className="workspace">
              <div className="canvas">
                <div className="boundary">
                  <span>{text.boundary}</span>
                </div>
                <div className="frame-host" ref={host} />
                {(render === 'navigation' || render === 'failed') && (
                  <div className="notice" role="alert">
                    <h2>{text.blocked}</h2>
                    <p>{render === 'navigation' ? text.navigation : text.failed}</p>
                    <p>{text.limit}</p>
                  </div>
                )}
              </div>
            </div>
            <p className="isolation-note">{text.warning}</p>
          </section>
        ) : (
          <section className="notice" role={state.kind === 'opening' ? 'status' : 'alert'}>
            <h1>
              {state.kind === 'opening'
                ? text.readerOpening
                : state.kind === 'ended'
                  ? text.readerEnded
                  : text.error}
            </h1>
            {state.kind === 'ended' && <p>{text.readerEndedNote}</p>}
            {state.kind === 'invalid' && <p>{text.readerInvalid}</p>}
            {state.kind === 'failed' && <p>{text.readerFailed}</p>}
          </section>
        )}
      </main>
      <footer>{text.readerNote}</footer>
    </>
  );
}
