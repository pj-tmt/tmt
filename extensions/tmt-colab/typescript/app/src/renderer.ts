import { text } from './strings.js';

/** Exact HTML source byte limit, owned by colab-v1 Resource bounds. */
export const MAX_RENDER_SOURCE_BYTES = 2 * 1024 * 1024;
export const RENDER_CSP =
  "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; object-src 'none'; frame-src 'none'; font-src 'none'; media-src 'none'; worker-src 'none'; manifest-src 'none'";
export type RenderState = 'ready' | 'navigation' | 'failed';
export interface RenderSnapshot {
  readonly renderId: string;
  readonly sourceDigest: string;
  readonly source: string;
}

export async function captureRender(source: string): Promise<RenderSnapshot> {
  const bytes = new TextEncoder().encode(source);
  if (bytes.byteLength > MAX_RENDER_SOURCE_BYTES) throw new Error('Page exceeds preview limit');
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  const sourceDigest = [...new Uint8Array(digest)]
    .map((v) => v.toString(16).padStart(2, '0'))
    .join('');
  return Object.freeze({
    renderId: `${crypto.randomUUID()}:${sourceDigest}`,
    sourceDigest,
    source,
  });
}

/** Only plaintext HTML and render metadata enter the opaque frame. Page scripts
 * can impersonate its bootstrap: this handshake grants no application authority. */
export async function mountRenderer(
  host: HTMLElement,
  source: string,
  options: {
    signal: AbortSignal;
    onState(state: RenderState): void;
  },
): Promise<{ readonly snapshot: RenderSnapshot; destroy(): void }> {
  const snapshot = await captureRender(source);
  options.signal.throwIfAborted();
  const frame = document.createElement('iframe');
  frame.title = text.boundary;
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.referrerPolicy = 'no-referrer';
  frame.dataset.renderId = snapshot.renderId;
  frame.dataset.sourceDigest = snapshot.sourceDigest;
  let stopped = false,
    loads = 0;
  const channel = new MessageChannel();
  const destroy = () => {
    if (stopped) return;
    stopped = true;
    clearTimeout(deadline);
    window.removeEventListener('message', bound);
    options.signal.removeEventListener('abort', destroy);
    frame.onload = null;
    channel.port1.close();
    channel.port2.close();
    frame.remove();
  };
  const stop = (state: RenderState) => {
    destroy();
    options.onState(state);
  };
  const bound = (event: MessageEvent<unknown>) => {
    if (stopped || event.source !== frame.contentWindow) return;
    const data = event.data;
    if (!data || typeof data !== 'object' || Array.isArray(data)) return;
    const value = data as Record<string, unknown>;
    if (
      Object.keys(value).length !== 2 ||
      value.type !== 'colab.render.bound' ||
      value.renderId !== snapshot.renderId
    )
      return;
    clearTimeout(deadline);
    window.removeEventListener('message', bound);
    options.onState('ready');
  };
  // No port inbound commands are implemented in this slice; unknown traffic has no effects.
  channel.port1.onmessage = () => {};
  const deadline = setTimeout(() => stop('failed'), 5000);
  window.addEventListener('message', bound);
  options.signal.addEventListener('abort', destroy, { once: true });
  frame.onload = () => {
    if (stopped) return;
    if (++loads !== 1) {
      stop('navigation');
      return;
    }
    frame.contentWindow?.postMessage(
      { type: 'colab.render.bind', renderId: snapshot.renderId },
      '*',
      [channel.port2],
    );
  };
  const bootstrap = `<script>addEventListener('message',e=>{if(e.source!==parent||e.data?.type!=='colab.render.bind'||e.data.renderId!==${JSON.stringify(snapshot.renderId)}||e.ports.length!==1)return;parent.postMessage({type:'colab.render.bound',renderId:${JSON.stringify(snapshot.renderId)}},'*')},{once:true})</script>`;
  frame.srcdoc = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${RENDER_CSP}">${bootstrap}</head><body>${snapshot.source}</body></html>`;
  host.replaceChildren(frame);
  return { snapshot, destroy };
}
