import { validateSelector, type QuoteSelector } from './thread-records.js';
import { text } from './strings.js';
import { getTheme, subscribeTheme } from './theme.js';
import { switcherModifiers, type SwitcherKeyRequest } from './quick-switcher-key.js';
import {
  DocumentImages,
  IMAGE_REFERENCE,
  IMAGE_REFERENCES,
  type DocumentImageInput,
} from './document-images.js';

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
/** Cosmetic geometry only, shared by selection and saved-anchor reports. */
export function isRenderRectangle(value: unknown): value is SelectionRect {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const rect = value as Record<string, unknown>;
  return (
    Object.keys(rect).length === 4 &&
    ['x', 'y', 'width', 'height'].every(
      (key) =>
        typeof rect[key] === 'number' &&
        Number.isFinite(rect[key]) &&
        Math.abs(rect[key]) <= MAX_RENDER_HEIGHT,
    ) &&
    (rect.width as number) >= 0 &&
    (rect.height as number) >= 0
  );
}
export interface AnchorPosition {
  id: string;
  top: number;
  beforeTop?: number;
  descriptionHeight?: number;
}
export interface InlineSlot {
  id: string;
  height: number;
  before?: number;
}
export interface QuestionRenderGroup {
  id: string;
  title: string | null;
  questions: string[];
  active: string;
}
export interface QuestionAnchor {
  id: string;
  selector: QuoteSelector;
  threads: string[];
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
    onToggleSelectionMode?(): void;
    /** Closed-set cosmetic key request; only trusted chrome decides its admitted action. */
    onKey?(key: 'Escape' | '@' | SwitcherKeyRequest): void;
    onOpenThread?(id: string): void;
    /** Checked is true only for an admitted resolution response, false while pending. */
    onAnchors?(resolved: string[], checked?: boolean): void;
    /** Trusted chrome's fixed header inset; absent for standalone renderer probes. */
    viewportInset?(): number;
    onSlots?(slots: AnchorPosition[]): void;
    onQuestions?(questions: QuestionAnchor[], checked?: boolean): void;
    images?: DocumentImageInput;
  },
): Promise<{
  readonly snapshot: RenderSnapshot;
  highlight(
    anchors: { id: string; selector: QuoteSelector; ordinal: number }[],
    draft?: QuoteSelector,
    threadCount?: number,
  ): void;
  slots(slots: InlineSlot[]): void;
  questions(
    blocks: { id: string; title: string }[],
    threads: { id: string; selector: QuoteSelector }[],
    groups: QuestionRenderGroup[],
  ): void;
  scrollAnchor(id: string): void;
  selectionMode(enabled: boolean): void;
  images(input: DocumentImageInput): void;
  /** Cosmetic position already admitted from this render; never discussion authority. */
  anchorRectangle(id: string): SelectionRect | undefined;
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
  let imageInput = options.images ?? { descriptors: [] };
  let imageRequestId = '';
  let imageResolver: DocumentImages | undefined;
  const images = (input: DocumentImageInput) => {
    imageInput = input;
    imageResolver?.close();
    if (stopped || !ready) return;
    imageRequestId = crypto.randomUUID();
    const current = imageRequestId;
    imageResolver = new DocumentImages(input, () => !stopped && current === imageRequestId);
    channel.port1.postMessage({
      type: 'colab.render.images',
      renderId: snapshot.renderId,
      requestId: current,
    });
  };
  const sendTheme = () => {
    if (stopped || !ready) return;
    channel.port1.postMessage({
      type: 'colab.render.theme',
      renderId: snapshot.renderId,
      theme: getTheme(),
    });
  };
  const stopTheme = subscribeTheme(sendTheme);
  let requestId = '';
  let anchors: { id: string; selector: QuoteSelector; ordinal?: number }[] = [];
  const positions = new Map<string, { top: number; rect: SelectionRect }>();
  let questionRequestId = '';
  let questionBlocks: { id: string; title: string }[] = [];
  let questionThreads: { id: string; selector: QuoteSelector }[] = [];
  let questionGroups: QuestionRenderGroup[] = [];
  let slotRequestId = '';
  let slotHeights: InlineSlot[] = [];
  let slotRounds = 0;
  let slotPositions = '';
  let lastSlots: AnchorPosition[] = [];
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
    options.onSlots?.([]);
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
    slotRounds = 0;
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
    imageResolver?.close();
    if (ready)
      channel.port1.postMessage({
        type: 'colab.render.images',
        renderId: snapshot.renderId,
        requestId: crypto.randomUUID(),
      });
    stopped = true;
    stopTheme();
    options.onSlots?.([]);
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
      Object.keys(value).length === 3 &&
      value.type === 'colab.render.key' &&
      value.renderId === snapshot.renderId &&
      (value.key === 'Escape' || value.key === '@')
    ) {
      options.onKey?.(value.key);
      return;
    }
    if (
      ready &&
      Object.keys(value).length === 7 &&
      value.type === 'colab.render.key' &&
      value.renderId === snapshot.renderId &&
      value.key === 'K' &&
      typeof value.ctrlKey === 'boolean' &&
      typeof value.metaKey === 'boolean' &&
      value.altKey === false &&
      value.shiftKey === false &&
      switcherModifiers({
        ctrlKey: value.ctrlKey,
        metaKey: value.metaKey,
        altKey: false,
        shiftKey: false,
      })
    ) {
      options.onKey?.({
        key: 'K',
        ctrlKey: value.ctrlKey,
        metaKey: value.metaKey,
        altKey: false,
        shiftKey: false,
      });
      return;
    }
    if (
      ready &&
      Object.keys(value).length === 2 &&
      value.renderId === snapshot.renderId &&
      (value.type === 'colab.render.annotate' || value.type === 'colab.render.toggle-selection')
    ) {
      if (value.type === 'colab.render.toggle-selection') options.onToggleSelectionMode?.();
      else options.onAnnotate?.();
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
        if (!isRenderRectangle(value.rect)) return;
        if (selector) rect = structuredClone(value.rect);
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
    // A choice can change between source init and the bound acknowledgement.
    sendTheme();
    images(imageInput);
    options.onState('ready');
  };
  channel.port1.onmessage = (event: MessageEvent<unknown>) => {
    if (stopped || !ready) return;
    const value = event.data as Record<string, unknown> | null;
    if (value?.type === 'colab.render.images') {
      if (
        Object.keys(value).length !== 4 ||
        value.renderId !== snapshot.renderId ||
        value.requestId !== imageRequestId ||
        !Array.isArray(value.references) ||
        value.references.length > IMAGE_REFERENCES ||
        value.references.some(
          (reference: unknown) => typeof reference !== 'string' || !IMAGE_REFERENCE.test(reference),
        )
      )
        return;
      const current = imageRequestId;
      void imageResolver?.resolve(value.references, (image) => {
        if (!stopped && current === imageRequestId)
          channel.port1.postMessage({
            type: 'colab.render.image',
            renderId: snapshot.renderId,
            requestId: current,
            ...image,
          });
      });
      return;
    }
    if (value?.type === 'colab.render.questions') {
      if (
        Object.keys(value).length !== 4 ||
        value.renderId !== snapshot.renderId ||
        value.requestId !== questionRequestId ||
        !Array.isArray(value.questions) ||
        value.questions.length > questionBlocks.length
      )
        return;
      const seen = new Set<string>();
      const admittedThreads = new Set(questionThreads.map((thread) => thread.id));
      const result: QuestionAnchor[] = [];
      try {
        for (const item of value.questions) {
          const block = questionBlocks.find((block) => block.id === item?.id);
          if (
            !block ||
            Object.keys(item).length !== 3 ||
            seen.has(item.id) ||
            !Array.isArray(item.threads) ||
            item.threads.length > questionThreads.length ||
            item.threads.some(
              (id: unknown) => typeof id !== 'string' || !admittedThreads.has(id),
            ) ||
            new Set(item.threads).size !== item.threads.length
          )
            return;
          validateSelector(item.selector);
          if (item.selector.exact !== block.title) return;
          seen.add(item.id);
          result.push({
            id: item.id,
            selector: structuredClone(item.selector),
            threads: [...item.threads],
          });
        }
      } catch {
        return;
      }
      options.onQuestions?.(result, true);
      return;
    }
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      value.type === 'colab.render.slots'
    ) {
      if (
        innerScroll ||
        Object.keys(value).length !== 4 ||
        value.renderId !== snapshot.renderId ||
        value.requestId !== slotRequestId ||
        !Array.isArray(value.slots) ||
        value.slots.length > slotHeights.length
      )
        return;
      const admitted = new Set(slotHeights.map(({ id }) => id));
      const seen = new Set<string>();
      const next: AnchorPosition[] = [];
      for (const slot of value.slots) {
        if (
          !slot ||
          typeof slot !== 'object' ||
          Array.isArray(slot) ||
          (Object.keys(slot).length !== 2 && Object.keys(slot).length !== 4) ||
          typeof slot.id !== 'string' ||
          !admitted.has(slot.id) ||
          seen.has(slot.id) ||
          typeof slot.top !== 'number' ||
          !Number.isFinite(slot.top) ||
          slot.top < 0 ||
          slot.top > MAX_RENDER_HEIGHT ||
          (Object.keys(slot).length === 4 &&
            (slotHeights.find((item) => item.id === slot.id)?.before === undefined ||
              !Number.isFinite(slot.beforeTop) ||
              slot.beforeTop < 0 ||
              slot.beforeTop > slot.top ||
              !Number.isFinite(slot.descriptionHeight) ||
              slot.descriptionHeight < 0 ||
              slot.descriptionHeight > 2000 ||
              Object.keys(slot).some(
                (key) => !['id', 'top', 'beforeTop', 'descriptionHeight'].includes(key),
              )))
        )
          return;
        seen.add(slot.id);
        next.push({
          id: slot.id,
          top: slot.top,
          ...(Object.keys(slot).length === 4
            ? { beforeTop: slot.beforeTop, descriptionHeight: slot.descriptionHeight }
            : {}),
        });
      }
      // A disappeared slot is detached even after the convergence budget is used.
      if (next.length === 0) {
        lastSlots = [];
        options.onSlots?.([]);
        return;
      }
      const serialized = JSON.stringify(next);
      if (serialized === slotPositions) return;
      if (slotRounds >= 3) {
        lastSlots = lastSlots.filter((slot) => seen.has(slot.id));
        options.onSlots?.(lastSlots);
        return;
      }
      slotRounds++;
      slotPositions = serialized;
      lastSlots = next;
      options.onSlots?.(next);
      return;
    }
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
          Object.keys(position).length !== 3 ||
          !isRenderRectangle(position.rect) ||
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
        positions.set(position.id, { top: position.top, rect: structuredClone(position.rect) });
      }
    }
    options.onAnchors?.([...value.resolved] as string[], true);
  };
  const highlight = (
    input: { id: string; selector: QuoteSelector; ordinal: number }[],
    draft?: QuoteSelector,
    threadCount = input.length,
  ) => {
    if (stopped) return;
    // Author code can inspect everything delivered into its frame. Rebuild this
    // narrow view instead of forwarding caller objects or discussion labels.
    const next = input.map(({ id, selector, ordinal }) => ({
      id,
      selector: structuredClone(selector),
      ordinal,
    }));
    if (
      !Number.isSafeInteger(threadCount) ||
      threadCount < 0 ||
      next.some(
        ({ ordinal }) => !Number.isSafeInteger(ordinal) || ordinal < 1 || ordinal > threadCount,
      ) ||
      new Set(next.map(({ ordinal }) => ordinal)).size !== next.length ||
      new Set(next.map(({ id }) => id)).size !== next.length
    )
      return;
    // The unsaved quote resolves separately, without a thread ordinal or action.
    if (draft) validateSelector(draft);
    if (
      next.length > 1000 ||
      next.some((v) => typeof v.id !== 'string' || v.id.length === 0 || v.id.length > 73)
    )
      return;
    for (const item of next) validateSelector(item.selector);
    positions.clear();
    requestId = crypto.randomUUID();
    const message = {
      type: 'colab.render.highlight',
      renderId: snapshot.renderId,
      requestId,
      anchors: next,
      threadCount,
      draft: draft ? structuredClone(draft) : null,
    };
    if (new TextEncoder().encode(JSON.stringify(message)).length > 256 * 1024) {
      anchors = [];
      options.onAnchors?.([]);
      channel.port1.postMessage({ ...message, anchors: [] });
      return;
    }
    anchors = [...next, ...(draft ? [{ id: '', selector: structuredClone(draft) }] : [])];
    options.onAnchors?.([]);
    channel.port1.postMessage(message);
  };
  const questions = (
    blocks: { id: string; title: string }[],
    threads: { id: string; selector: QuoteSelector }[],
    groups: QuestionRenderGroup[],
  ) => {
    if (
      stopped ||
      blocks.length > 50 ||
      groups.length > 50 ||
      new Set(groups.map((group) => group.id)).size !== groups.length ||
      groups.some(
        (group) =>
          !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(group.id) ||
          (group.title !== null &&
            (typeof group.title !== 'string' || !group.title || [...group.title].length > 200)) ||
          !group.questions.length ||
          group.questions.length > 12 ||
          !group.questions.includes(group.active) ||
          new Set(group.questions).size !== group.questions.length ||
          (group.title === null &&
            (group.questions.length !== 1 || group.id !== group.questions[0])) ||
          group.questions.some((id) => !blocks.some((block) => block.id === id)),
      ) ||
      groups.flatMap((group) => group.questions).length !== blocks.length ||
      new Set(groups.flatMap((group) => group.questions)).size !== blocks.length ||
      threads.length > 1000 ||
      new Set(blocks.map((block) => block.id)).size !== blocks.length ||
      blocks.some(
        (block) =>
          !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(block.id) ||
          [...block.title].length > 200,
      ) ||
      threads.some((thread) => typeof thread.id !== 'string' || thread.id.length > 73)
    )
      return;
    for (const thread of threads) validateSelector(thread.selector);
    const changed =
      JSON.stringify(questionBlocks) !== JSON.stringify(blocks) ||
      JSON.stringify(questionGroups.map(({ active: _active, ...group }) => group)) !==
        JSON.stringify(groups.map(({ active: _active, ...group }) => group));
    const moved = new Set(
      groups
        .filter(
          (group) =>
            questionGroups.find((previous) => previous.id === group.id)?.active !== group.active,
        )
        .map((group) => group.id),
    );
    if (moved.size) {
      slotRounds = 0;
      slotPositions = '';
    }
    questionGroups = groups.map(({ id, title, questions, active }) => ({
      id,
      title,
      questions: [...questions],
      active,
    }));
    questionBlocks = blocks.map(({ id, title }) => ({ id, title }));
    questionThreads = threads.map(({ id, selector }) => ({
      id,
      selector: structuredClone(selector),
    }));
    questionRequestId = crypto.randomUUID();
    const message = {
      type: 'colab.render.questions',
      renderId: snapshot.renderId,
      requestId: questionRequestId,
      blocks: questionBlocks,
      groups: questionGroups,
      threads: questionThreads,
    };
    if (changed) options.onQuestions?.([], false);
    if (new TextEncoder().encode(JSON.stringify(message)).length > 256 * 1024) return;
    channel.port1.postMessage(message);
    if (moved.size && slotHeights.length) {
      // A tab change needs geometry for that description. Retire its old answer geometry while
      // the existing slot owner requests fresh placement on the same ordered port.
      lastSlots = lastSlots.map((slot) => {
        if (!moved.has(slot.id)) return slot;
        // The strip keeps its admitted position/focus while answer bands detach.
        const { descriptionHeight: _descriptionHeight, ...placement } = slot;
        return placement;
      });
      slots(slotHeights);
    }
  };
  const slots = (input: InlineSlot[]) => {
    if (
      stopped ||
      input.length > 250 ||
      new Set(input.map((v) => v.id)).size !== input.length ||
      input.some(
        (v) =>
          !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(v.id) ||
          !Number.isFinite(v.height) ||
          (v.before !== undefined && !Number.isFinite(v.before)),
      )
    )
      return;
    const heights = input.map(({ id, height, before }) => ({
      id,
      height: Math.min(2000, Math.max(0, Math.ceil(height))),
      ...(before !== undefined ? { before: Math.min(2000, Math.max(0, Math.ceil(before))) } : {}),
    }));
    // Trusted card updates change the page's content height independently of its
    // viewport. Restart the growth baseline only for changed parent dimensions;
    // repeated requests and frame reports still share the bounded growth budget.
    if (JSON.stringify(heights) !== JSON.stringify(slotHeights)) growthReports = 0;
    slotHeights = heights;
    slotRequestId = crypto.randomUUID();
    slotRounds = 0;
    slotPositions = '';
    // Height changes retain admitted placement and focused controls until the
    // renderer confirms a slot vanished. A request alone is not detachment.
    lastSlots = lastSlots.filter(
      (slot) => !innerScroll && slotHeights.some(({ id }) => id === slot.id),
    );
    options.onSlots?.(lastSlots);
    channel.port1.postMessage({
      type: 'colab.render.slots',
      renderId: snapshot.renderId,
      requestId: slotRequestId,
      slots: innerScroll ? [] : slotHeights,
    });
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
    frame.contentWindow?.postMessage(
      { type: 'colab.render.bind', ...snapshot, theme: getTheme() },
      '*',
      [channel.port2],
    );
  };
  frame.src = new URL('./renderer.html', document.baseURI).href;
  // Preserve the current offset through replacement, without replaying an older
  // offset when a later height report arrives after the person has scrolled.
  const scroll =
    previousHeight === undefined ? undefined : { left: window.scrollX, top: window.scrollY };
  host.replaceChildren(frame);
  if (scroll) window.scrollTo({ ...scroll, behavior: 'instant' });
  const scrollAnchor = (id: string) => {
    const top = positions.get(id)?.top;
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
  const anchorRectangle = (id: string): SelectionRect | undefined => {
    const position = positions.get(id);
    if (stopped || !position) return;
    return { ...position.rect, ...(innerScroll ? { y: inset() } : {}) };
  };
  const selectionMode = (enabled: boolean) => {
    if (stopped || !ready) return;
    channel.port1.postMessage({
      type: 'colab.render.selection-mode',
      renderId: snapshot.renderId,
      enabled,
    });
  };
  return {
    snapshot,
    highlight,
    slots,
    questions,
    scrollAnchor,
    selectionMode,
    images,
    anchorRectangle,
    release,
    destroy,
  };
}
