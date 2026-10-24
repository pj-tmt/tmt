import { admitsOverlayEscape, outsideOverlay, restoreOverlayFocus } from './overlay.js';
import { claimBrowserPopup, placeBrowserPopup } from './popup.js';

export interface MenuEntry<Key extends string> {
  key: Key;
  label: string;
  disabled?: boolean;
}
export interface MenuActivationEvent {
  readonly isTrusted: boolean;
}
export interface MenuKeyboardEvent extends MenuActivationEvent {
  readonly key: string;
  readonly defaultPrevented: boolean;
  readonly repeat?: boolean;
  readonly keyCode?: number;
  readonly altKey?: boolean;
  readonly ctrlKey?: boolean;
  readonly metaKey?: boolean;
  readonly shiftKey?: boolean;
  readonly isComposing?: boolean;
  readonly nativeEvent?: { readonly isComposing?: boolean };
  preventDefault(): void;
  stopPropagation(): void;
}
interface SurfaceEvent extends MenuActivationEvent {
  readonly target: EventTarget | null;
  preventDefault(): void;
  stopPropagation(): void;
}
interface PointerEvent extends SurfaceEvent {
  readonly clientX: number;
  readonly clientY: number;
}
interface MenuSnapshot {
  readonly open: boolean;
  readonly present: boolean;
}
export interface MenuControllerOptions<Key extends string, Event extends MenuActivationEvent> {
  items(): readonly MenuEntry<Key>[];
  disabled(): boolean;
  hasDescription(): boolean;
  trigger(): HTMLButtonElement | null;
  surface(): HTMLDialogElement | null;
  entries(): readonly (HTMLButtonElement | null)[];
  selection(): (key: Key, event: Event) => void;
  onOpenChange(open: boolean): void;
}

/** One transient menu owner for React and framework-free renderers; commands stay with callers. */
export function createMenuController<Key extends string, Event extends MenuActivationEvent>(
  options: MenuControllerOptions<Key, Event>,
) {
  let snapshot: MenuSnapshot = { open: false, present: false };
  const listeners = new Set<() => void>();
  let first: 'first' | 'last' = 'first';
  let search = { text: '', at: 0 };
  let focused: Key | undefined;
  let reported = false;
  let disposed = false;
  let detach: (() => void) | undefined;
  let cancelExit: (() => void) | undefined;
  const enabled = () => options.items().flatMap((item, index) => (item.disabled ? [] : [index]));
  const publish = (open: boolean, present: boolean) => {
    if (snapshot.open === open && snapshot.present === present) return;
    snapshot = { open, present };
    for (const notify of listeners) notify();
  };
  const report = (open: boolean) => {
    if (reported === open) return;
    reported = open;
    options.onOpenChange(open);
  };
  const canOpen = () =>
    !disposed && !options.disabled() && (enabled().length > 0 || options.hasDescription());
  function open(edge: 'first' | 'last' = 'first') {
    if (!canOpen() || snapshot.open) return;
    cancelExit?.();
    cancelExit = undefined;
    first = edge;
    focused = undefined;
    search = { text: '', at: 0 };
    publish(true, true);
  }
  function close(refocus = true) {
    if (!snapshot.open) return;
    const node = options.surface();
    publish(false, snapshot.present);
    if (node) {
      node.dataset.state = 'closed';
      if (node.open) node.close();
    }
    detach?.();
    report(false);
    if (refocus) restoreOverlayFocus(options.trigger());
    const animations = node?.getAnimations?.() ?? [];
    if (!animations.length) {
      publish(false, false);
      return;
    }
    let current = true;
    cancelExit = () => {
      current = false;
    };
    void Promise.allSettled(animations.map((animation) => animation.finished)).then(() => {
      if (current && !snapshot.open && !disposed) {
        cancelExit = undefined;
        publish(false, false);
      }
    });
  }
  function attachSurface(): (() => void) | undefined {
    const node = options.surface();
    const anchor = options.trigger();
    if (!snapshot.open || !node || !anchor) return;
    if (!node.isConnected || !anchor.isConnected || !anchor.getClientRects().length) {
      close(false);
      return;
    }
    if (detach) return detach;
    const release = claimBrowserPopup(anchor.ownerDocument, () => close(false));
    let position: (() => void) | undefined;
    const cleanup = () => {
      if (detach !== cleanup) return;
      detach = undefined;
      position?.();
      release();
      if (node.open) node.close();
      // React effect replay keeps the node connected; only actual removal owes a close.
      queueMicrotask(() => {
        const current = options.surface();
        if (!node.isConnected && snapshot.open && (!current || current === node)) close(false);
      });
    };
    detach = cleanup;
    try {
      node.showModal();
      position = placeBrowserPopup(anchor, node, () => close(false));
      if (!snapshot.open) return cleanup;
      report(true);
      if (!snapshot.open) return cleanup;
      const choices = enabled();
      const index = first === 'last' ? choices.at(-1) : choices[0];
      focused = index === undefined ? undefined : options.items()[index]?.key;
      (index === undefined ? node : (options.entries()[index] ?? node)).focus();
    } catch (error) {
      cleanup();
      close(false);
      throw error;
    }
    return cleanup;
  }
  function refresh() {
    const node = options.surface();
    if (!snapshot.open || !node) return;
    const active = node.ownerDocument.activeElement;
    if (active === node) return;
    if (active !== node.ownerDocument.body && (!active || !node.contains(active))) return;
    const entries = options.entries();
    const choices = enabled();
    if (choices.some((index) => entries[index] === active)) return;
    const remembered = choices.find((index) => options.items()[index]!.key === focused);
    const next = remembered ?? choices[0];
    (next === undefined ? node : (entries[next] ?? node)).focus();
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(notify: () => void) {
      listeners.add(notify);
      return () => {
        listeners.delete(notify);
      };
    },
    canOpen,
    open,
    close,
    attachSurface,
    refresh,
    focused(key: Key) {
      if (snapshot.open) focused = key;
    },
    triggerKey(event: MenuKeyboardEvent) {
      if (
        !['ArrowDown', 'ArrowUp'].includes(event.key) ||
        !event.isTrusted ||
        (event.nativeEvent?.isComposing ?? event.isComposing)
      )
        return;
      event.preventDefault();
      open(event.key === 'ArrowUp' ? 'last' : 'first');
    },
    menuKey(event: MenuKeyboardEvent) {
      if (!snapshot.open || event.defaultPrevented) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        if (admitsOverlayEscape(event)) close();
        return;
      }
      if (
        (event.nativeEvent?.isComposing ?? event.isComposing) ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey
      )
        return;
      if (event.key === 'Tab') {
        close();
        return;
      }
      const choices = enabled();
      const entries = options.entries();
      const at = choices.indexOf(
        entries.findIndex((node) => node === node?.ownerDocument.activeElement),
      );
      let next: number | undefined;
      if (event.key === 'ArrowDown') next = choices[(at + 1) % choices.length];
      else if (event.key === 'ArrowUp') next = choices[(at - 1 + choices.length) % choices.length];
      else if (event.key === 'Home') next = choices[0];
      else if (event.key === 'End') next = choices.at(-1);
      else if (event.key.length === 1 && event.key !== ' ') {
        const now = performance.now();
        search = {
          text: (now - search.at > 700 ? '' : search.text) + event.key.toLocaleLowerCase(),
          at: now,
        };
        next = [...choices.slice(at + 1), ...choices.slice(0, at + 1)].find((index) =>
          options.items()[index]!.label.toLocaleLowerCase().startsWith(search.text),
        );
      } else return;
      event.preventDefault();
      if (next !== undefined) {
        focused = options.items()[next]?.key;
        entries[next]?.focus();
      }
    },
    select(key: Key, event: Event) {
      const item = options.items().find((item) => item.key === key);
      if (!snapshot.open || !event.isTrusted || !item || item.disabled) return;
      const select = options.selection();
      close();
      select(key, event);
    },
    nativeClose(event: Pick<SurfaceEvent, 'target' | 'stopPropagation'>) {
      event.stopPropagation();
      const node = options.surface();
      if (snapshot.open && event.target === node && !node?.open) close();
    },
    cancel(event: SurfaceEvent) {
      event.preventDefault();
      event.stopPropagation();
      if (event.isTrusted) close();
    },
    pointerDown(event: PointerEvent) {
      const node = options.surface();
      if (node && event.target === node && outsideOverlay(node, event.clientX, event.clientY))
        event.preventDefault();
    },
    surfaceClick(event: PointerEvent) {
      const node = options.surface();
      if (!node || event.target !== node || !outsideOverlay(node, event.clientX, event.clientY))
        return;
      event.preventDefault();
      event.stopPropagation();
      if (event.isTrusted) close();
    },
    dispose() {
      if (disposed) return;
      close(false);
      detach?.();
      cancelExit?.();
      cancelExit = undefined;
      disposed = true;
      publish(false, false);
      listeners.clear();
    },
  };
}
