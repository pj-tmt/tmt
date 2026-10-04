import { validateSelector, type QuoteSelector } from './thread-records.js';
import { text } from './strings.js';

/** Exact HTML source byte limit, owned by colab-v1 Resource bounds. */
export const MAX_RENDER_SOURCE_BYTES = 2 * 1024 * 1024;
export const MAX_SELECTION_BYTES = 16 * 1024;
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
    onSelection?(text: string, selector?: QuoteSelector | null): void;
    onAnchors?(resolved: string[]): void;
  },
): Promise<{
  readonly snapshot: RenderSnapshot;
  highlight(anchors: { id: string; selector: QuoteSelector }[]): void;
  destroy(): void;
}> {
  const snapshot = await captureRender(source);
  options.signal.throwIfAborted();
  const frame = document.createElement('iframe');
  frame.title = text.boundary;
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.referrerPolicy = 'no-referrer';
  frame.dataset.renderId = snapshot.renderId;
  frame.dataset.sourceDigest = snapshot.sourceDigest;
  let stopped = false,
    loads = 0,
    ready = false;
  const channel = new MessageChannel();
  let requestId = '';
  let anchors: { id: string; selector: QuoteSelector }[] = [];
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
    options.onSelection?.('', null);
    options.onAnchors?.([]);
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
      ready &&
      (Object.keys(value).length === 3 ||
        (Object.keys(value).length === 4 && Object.hasOwn(value, 'selector'))) &&
      Object.keys(value).every((key) => ['type', 'renderId', 'text', 'selector'].includes(key)) &&
      value.type === 'colab.render.selection' &&
      value.renderId === snapshot.renderId &&
      typeof value.text === 'string' &&
      value.text.length <= MAX_SELECTION_BYTES &&
      new TextEncoder().encode(value.text).length <= MAX_SELECTION_BYTES
    ) {
      // Frame claims are untrusted, including claims from the bootstrap.
      let selector: QuoteSelector | null = null;
      if (Object.hasOwn(value, 'selector') && value.selector !== null) {
        try {
          validateSelector(value.selector);
          if (value.selector.exact !== value.text) return;
          selector = structuredClone(value.selector);
        } catch {
          return;
        }
      }
      options.onSelection?.(value.text, selector);
      return;
    }
    if (
      ready ||
      Object.keys(value).length !== 2 ||
      value.type !== 'colab.render.bound' ||
      value.renderId !== snapshot.renderId
    )
      return;
    clearTimeout(deadline);
    ready = true;
    options.onState('ready');
  };
  channel.port1.onmessage = (event: MessageEvent<unknown>) => {
    if (stopped || !ready) return;
    const value = event.data as Record<string, unknown> | null;
    if (
      !value ||
      typeof value !== 'object' ||
      Array.isArray(value) ||
      Object.keys(value).length !== 4 ||
      value.type !== 'colab.render.anchors' ||
      value.renderId !== snapshot.renderId ||
      value.requestId !== requestId ||
      !Array.isArray(value.resolved) ||
      value.resolved.length > anchors.length
    )
      return;
    const ids = new Set(anchors.map((v) => v.id));
    if (
      value.resolved.some((id) => typeof id !== 'string' || !ids.has(id)) ||
      new Set(value.resolved).size !== value.resolved.length
    )
      return;
    options.onAnchors?.([...value.resolved] as string[]);
  };
  const highlight = (input: { id: string; selector: QuoteSelector }[]) => {
    if (stopped) return;
    const next = structuredClone(input);
    if (next.length > 1000 || next.some((v) => typeof v.id !== 'string' || v.id.length > 73))
      return;
    for (const item of next) validateSelector(item.selector);
    requestId = crypto.randomUUID();
    const message = {
      type: 'colab.render.highlight',
      renderId: snapshot.renderId,
      requestId,
      anchors: next,
    };
    if (new TextEncoder().encode(JSON.stringify(message)).length > 256 * 1024) {
      anchors = [];
      options.onAnchors?.([]);
      channel.port1.postMessage({ ...message, anchors: [] });
      return;
    }
    anchors = next;
    options.onAnchors?.([]);
    channel.port1.postMessage(message);
  };
  const deadline = setTimeout(() => stop('failed'), 5000);
  window.addEventListener('message', bound);
  options.signal.addEventListener('abort', destroy, { once: true });
  frame.onload = () => {
    if (stopped) return;
    if (++loads > 2) {
      stop('navigation');
      return;
    }
    // Source init runs in a later postMessage task, outside iframe load-in-progress,
    // so document.open does not mute the completion load. HTML's "the end" fires
    // Window load then completes the document through the iframe load event steps:
    // https://html.spec.whatwg.org/multipage/parsing.html#the-end
    // https://html.spec.whatwg.org/multipage/iframe-embed-object.html#iframe-load-event-steps
    // The bootstrap load receives source once; document.write completes the second load.
    if (loads !== 1) return;
    frame.contentWindow?.postMessage({ type: 'colab.render.bind', ...snapshot }, '*', [
      channel.port2,
    ]);
  };
  frame.src = new URL('./renderer.html', document.baseURI).href;
  host.replaceChildren(frame);
  return { snapshot, highlight, destroy };
}
