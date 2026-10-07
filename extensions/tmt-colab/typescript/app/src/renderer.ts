import { validateSelector, type QuoteSelector } from './thread-records.js';
import { text } from './strings.js';

/** Exact HTML source byte limit, owned by colab-v1 Resource bounds. */
export const MAX_RENDER_SOURCE_BYTES = 2 * 1024 * 1024;
export const MAX_SELECTION_BYTES = 16 * 1024;
/** Cosmetic layout claims never allocate an unbounded frame or resize it faster than 10 Hz. */
export const MAX_RENDER_HEIGHT = 1_000_000;
const HEIGHT_UPDATE_MS = 100;
const GROWTH_REPORT_LIMIT = 5;
export interface SelectionRect {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface AnchorPosition {
  id: string;
  top: number;
}
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
    onSelection?(text: string, selector?: QuoteSelector | null, rect?: SelectionRect | null): void;
    onAnnotate?(): void;
    onOpenThread?(id: string): void;
    /** Checked is true only for an admitted resolution response, false while pending. */
    onAnchors?(resolved: string[], checked?: boolean): void;
    /** Trusted chrome's fixed header inset; absent for standalone renderer probes. */
    viewportInset?(): number;
  },
): Promise<{
  readonly snapshot: RenderSnapshot;
  highlight(anchors: { id: string; selector: QuoteSelector }[], draft?: QuoteSelector): void;
  scrollAnchor(id: string): void;
  /** Stop the old channel while its frame preserves layout until replacement. */
  release(): void;
  destroy(): void;
}> {
  // Keep the old layout while the replacement loads, so the browser does not
  // clamp the window offset to a temporary viewport-high document.
  const previousHeight = host.querySelector('iframe')?.getBoundingClientRect().height;
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
  const positions = new Map<string, number>();
  const inset = () => Math.max(0, Math.min(window.innerHeight, options.viewportInset?.() ?? 0));
  const viewportHeight = () => Math.max(1, window.innerHeight - inset());
  let reportedHeight = 0,
    pendingHeight: number | undefined,
    heightTimer: ReturnType<typeof setTimeout> | undefined,
    updatedAt = -Infinity,
    growingAt = 0,
    growthReports = 0,
    lastHeight = Math.min(MAX_RENDER_HEIGHT, Math.max(viewportHeight(), previousHeight ?? 0)),
    innerScroll = false,
    scrolledAt = -Infinity;
  frame.scrolling = 'no';
  frame.dataset.scrollMode = 'window';
  frame.style.height = `${lastHeight}px`;
  const fallback = () => {
    innerScroll = true;
    pendingHeight = undefined;
    clearTimeout(heightTimer);
    heightTimer = undefined;
    frame.scrolling = 'auto';
    frame.dataset.scrollMode = 'frame';
    frame.style.height = `${viewportHeight()}px`;
    channel.port1.postMessage({
      type: 'colab.render.layout',
      renderId: snapshot.renderId,
      mode: 'frame',
    });
  };
  const resize = () => {
    lastHeight = innerScroll ? viewportHeight() : Math.max(viewportHeight(), reportedHeight);
    growthReports = 0;
    frame.style.height = `${lastHeight}px`;
  };
  const applyHeight = () => {
    heightTimer = undefined;
    if (stopped || innerScroll || pendingHeight === undefined) return;
    const now = performance.now(),
      claimed = pendingHeight,
      next = Math.max(viewportHeight(), claimed);
    pendingHeight = undefined;
    // Repeated growth after sizing the frame is a viewport-coupled page. A bounded
    // cosmetic fallback is preferable to an indefinitely growing browser document.
    if (next > lastHeight + 1) {
      if (now - growingAt > 2000) {
        growthReports = 0;
        growingAt = now;
      }
      if (!growthReports) growingAt = now;
      if (++growthReports >= GROWTH_REPORT_LIMIT) {
        fallback();
        return;
      }
    } else growthReports = 0;
    reportedHeight = claimed;
    lastHeight = next;
    updatedAt = now;
    frame.style.height = `${next}px`;
  };
  const release = () => {
    if (stopped) return;
    stopped = true;
    clearTimeout(deadline);
    clearTimeout(heightTimer);
    window.removeEventListener('resize', resize);
    window.removeEventListener('message', bound);
    options.signal.removeEventListener('abort', destroy);
    frame.onload = null;
    channel.port1.close();
    channel.port2.close();
  };
  const destroy = () => {
    const mounted = frame.parentNode === host;
    release();
    frame.remove();
    if (mounted) {
      options.onSelection?.('', null);
      options.onAnchors?.([]);
    }
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
    if (ready && value.renderId === snapshot.renderId && Object.keys(value).length === 3) {
      if (
        value.type === 'colab.render.height' &&
        typeof value.height === 'number' &&
        Number.isFinite(value.height) &&
        value.height > 0
      ) {
        if (innerScroll) return;
        pendingHeight = Math.min(MAX_RENDER_HEIGHT, Math.ceil(value.height));
        const remaining = HEIGHT_UPDATE_MS - (performance.now() - updatedAt);
        if (remaining <= 0) applyHeight();
        else if (heightTimer === undefined) heightTimer = setTimeout(applyHeight, remaining);
        return;
      }
      if (
        value.type === 'colab.render.anchor' &&
        typeof value.top === 'number' &&
        Number.isFinite(value.top) &&
        value.top >= 0 &&
        value.top <= MAX_RENDER_HEIGHT
      ) {
        const now = performance.now();
        if (now - scrolledAt < HEIGHT_UPDATE_MS) return;
        scrolledAt = now;
        const top =
          frame.getBoundingClientRect().top +
          window.scrollY +
          (innerScroll ? 0 : value.top) -
          inset();
        window.scrollTo({
          top: Math.max(
            0,
            Math.min(document.documentElement.scrollHeight - window.innerHeight, top),
          ),
          behavior: 'instant',
        });
        return;
      }
    }
    if (
      ready &&
      Object.keys(value).length === 2 &&
      value.renderId === snapshot.renderId &&
      value.type === 'colab.render.annotate'
    ) {
      options.onAnnotate?.();
      return;
    }
    if (
      ready &&
      Object.keys(value).length >= 3 &&
      Object.keys(value).length <= 5 &&
      Object.keys(value).every((key) =>
        ['type', 'renderId', 'text', 'selector', 'rect'].includes(key),
      ) &&
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
      let rect: SelectionRect | null = null;
      if (value.rect !== undefined && value.rect !== null) {
        const candidate = value.rect as Record<string, unknown>;
        if (
          !candidate ||
          typeof candidate !== 'object' ||
          Array.isArray(candidate) ||
          Object.keys(candidate).length !== 4 ||
          !['x', 'y', 'width', 'height'].every(
            (key) =>
              typeof candidate[key] === 'number' &&
              Number.isFinite(candidate[key]) &&
              Math.abs(candidate[key] as number) <= MAX_RENDER_HEIGHT,
          ) ||
          (candidate.width as number) < 0 ||
          (candidate.height as number) < 0
        )
          return;
        if (selector)
          rect = {
            x: candidate.x as number,
            y: candidate.y as number,
            width: candidate.width as number,
            height: candidate.height as number,
          };
      }
      options.onSelection?.(value.text, selector, rect);
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
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      Object.keys(value).length === 4 &&
      value.type === 'colab.render.open-thread' &&
      value.renderId === snapshot.renderId &&
      value.requestId === requestId &&
      typeof value.id === 'string' &&
      value.id !== '' &&
      anchors.some((anchor) => anchor.id === value.id)
    ) {
      options.onOpenThread?.(value.id);
      return;
    }
    if (
      !value ||
      typeof value !== 'object' ||
      Array.isArray(value) ||
      ![4, 5].includes(Object.keys(value).length) ||
      Object.keys(value).some(
        (key) => !['type', 'renderId', 'requestId', 'resolved', 'positions'].includes(key),
      ) ||
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
    positions.clear();
    if (value.positions !== undefined) {
      if (!Array.isArray(value.positions) || value.positions.length > value.resolved.length) return;
      for (const position of value.positions) {
        if (
          !position ||
          typeof position !== 'object' ||
          Array.isArray(position) ||
          Object.keys(position).length !== 2 ||
          typeof position.id !== 'string' ||
          !value.resolved.includes(position.id) ||
          positions.has(position.id) ||
          typeof position.top !== 'number' ||
          !Number.isFinite(position.top) ||
          position.top < 0 ||
          position.top > MAX_RENDER_HEIGHT
        ) {
          positions.clear();
          return;
        }
        positions.set(position.id, position.top);
      }
    }
    options.onAnchors?.([...value.resolved] as string[], true);
  };
  const highlight = (input: { id: string; selector: QuoteSelector }[], draft?: QuoteSelector) => {
    if (stopped) return;
    // Author code can inspect everything delivered into its frame. Rebuild this
    // narrow view instead of forwarding caller objects or discussion labels.
    const next = input.map(({ id, selector }) => ({ id, selector: structuredClone(selector) }));
    // The empty ID checks an unsaved quote without adding a thread marker or action.
    if (draft) next.push({ id: '', selector: structuredClone(draft) });
    if (next.length > 1000 || next.some((v) => typeof v.id !== 'string' || v.id.length > 73))
      return;
    for (const item of next) validateSelector(item.selector);
    positions.clear();
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
  window.addEventListener('resize', resize);
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
  // Preserve the current offset through replacement, without replaying an older
  // offset when a later height report arrives after the person has scrolled.
  const scroll =
    previousHeight === undefined ? undefined : { left: window.scrollX, top: window.scrollY };
  host.replaceChildren(frame);
  if (scroll) window.scrollTo({ ...scroll, behavior: 'instant' });
  const scrollAnchor = (id: string) => {
    const top = positions.get(id);
    if (stopped || top === undefined) return;
    if (innerScroll)
      channel.port1.postMessage({
        type: 'colab.render.scroll-thread',
        renderId: snapshot.renderId,
        requestId,
        id,
      });
    window.scrollTo({
      top: Math.max(
        0,
        frame.getBoundingClientRect().top + window.scrollY + (innerScroll ? 0 : top) - inset(),
      ),
      behavior: 'instant',
    });
  };
  return { snapshot, highlight, scrollAnchor, release, destroy };
}
