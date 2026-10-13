import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { readFileSync } from 'node:fs';
import {
  useLayerController,
  useMenuLayerHandoff,
  useLayerFocus,
  focusLayerTarget,
  admitLayerDismiss,
  observeLayerViewport,
  type LayerControllerOptions,
} from '../src/ui/layer.js';
import { uiCandidates } from '../scripts/ui-source-policy.mjs';

// Callback/observer unit proofs only. Real Radix DOM/focus qualification is the browser gate.
const hooks = vi.hoisted(() => ({ index: 0, refs: [] as { current: unknown }[] }));
vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  useRef: (value: unknown) => (hooks.refs[hooks.index++] ??= { current: value }),
  useCallback: (callback: unknown) => callback,
}));
afterEach(() => {
  hooks.index = 0;
  hooks.refs = [];
  vi.unstubAllGlobals();
});
function rerender(options: LayerControllerOptions) {
  hooks.index = 0;
  return useLayerController(options);
}
function cancelable(type = 'close') {
  return new Event(type, { cancelable: true });
}

for (const reason of [
  'escape',
  'outside',
  'scrim',
  'close',
  'selection',
  'tab',
  'programmatic',
] as const) {
  it(`holds ${reason} on host veto and admits it once when the host permits`, () => {
    const changed = vi.fn();
    const event = cancelable();
    const denied = vi.fn(() => false);
    const controller = rerender({ open: true, onOpenChange: changed, canClose: denied });
    expect(controller.requestClose(reason, event)).toBe(false);
    expect(denied).toHaveBeenCalledWith({ reason, originalEvent: event });
    expect(changed).not.toHaveBeenCalled();
    const admitted = rerender({ open: true, onOpenChange: changed, canClose: () => true });
    expect(admitted.requestClose(reason, event)).toBe(true);
    expect(admitted.requestClose(reason, event)).toBe(false);
    expect(changed.mock.calls).toEqual([[false]]);
    rerender({ open: false, onOpenChange: changed });
    const reopened = rerender({ open: true, onOpenChange: changed });
    expect(reopened.requestClose(reason)).toBe(true);
    expect(changed.mock.calls).toEqual([[false], [false]]);
  });
}
it('keeps raw host state authoritative and recovers admission after a callback throws', () => {
  const changed = vi.fn(() => {
    throw new Error('host failure');
  });
  const first = rerender({ open: true, onOpenChange: changed });
  expect(() => first.requestClose()).toThrow('host failure');
  expect(() => first.requestClose()).toThrow('host failure');
  const veto = vi.fn(() => false);
  const closed = rerender({ open: false, onOpenChange: vi.fn(), canClose: veto });
  expect(closed.open).toBe(false);
  expect(veto).not.toHaveBeenCalled();
  expect(closed.requestClose()).toBe(false);
});
it('does not consume a child-prevented dismissal and preserves the original outside event', () => {
  const close = vi.fn(() => true);
  const controller = { open: true, requestOpen: vi.fn(), requestClose: close };
  const prevented = cancelable('keydown');
  prevented.preventDefault();
  admitLayerDismiss(prevented, controller, 'escape');
  expect(close).not.toHaveBeenCalled();
  const original = cancelable('pointerdown');
  const outside = new CustomEvent('outside', {
    cancelable: true,
    detail: { originalEvent: original },
  });
  admitLayerDismiss(outside, controller, 'outside');
  expect(outside.defaultPrevented).toBe(true);
  expect(close).toHaveBeenCalledWith('outside', original);
  const keyboard = Object.assign(cancelable('keydown'), { detail: 0 });
  admitLayerDismiss(keyboard, controller, 'escape');
  expect(close).toHaveBeenLastCalledWith('escape', keyboard);
});
it('a vetoed menu transfer does not open its target or suppress menu close autofocus', () => {
  const open = vi.fn();
  const changed = vi.fn();
  const menu = useLayerController({ open: true, onOpenChange: changed, canClose: () => false });
  const close = vi.spyOn(menu, 'requestClose');
  const layer = useLayerController({ open: false, onOpenChange: open });
  const handoff = useMenuLayerHandoff({
    menu,
    layer,
    trigger: () => null,
  });
  const select = cancelable('select');
  handoff.onSelect(select);
  expect(select.defaultPrevented).toBe(true);
  expect(close).toHaveBeenCalledWith('selection', select);
  expect(changed).not.toHaveBeenCalled();
  expect(open).not.toHaveBeenCalled();
  const autofocus = cancelable('autofocus');
  handoff.onCloseAutoFocus(autofocus);
  expect(autofocus.defaultPrevented).toBe(false);
  expect(handoff.returnFocus()).toBeNull();
});
it('an admitted menu transfer opens once and suppresses only the competing menu autofocus', () => {
  const close = vi.fn();
  const open = vi.fn();
  const trigger = {} as HTMLElement;
  const menu = useLayerController({ open: true, onOpenChange: close });
  const layer = useLayerController({ open: false, onOpenChange: open });
  const handoff = useMenuLayerHandoff({
    menu,
    layer,
    trigger: () => trigger,
  });
  handoff.onSelect(cancelable('select'));
  handoff.onSelect(cancelable('select'));
  expect(open).toHaveBeenCalledOnce();
  expect(open).toHaveBeenCalledWith(true);
  expect(close.mock.calls).toEqual([[false]]);
  const autofocus = cancelable('autofocus');
  handoff.onCloseAutoFocus(autofocus);
  expect(autofocus.defaultPrevented).toBe(true);
  const next = cancelable('autofocus');
  handoff.onCloseAutoFocus(next);
  expect(next.defaultPrevented).toBe(false);
  expect(handoff.returnFocus()).toBe(trigger);
  expect(handoff.returnFocus()).toBeNull();
});

class FocusTarget {
  isConnected = true;
  visible = true;
  inert = false;
  disabled = false;
  ownerDocument: { activeElement: FocusTarget | null };
  constructor(document: { activeElement: FocusTarget | null }) {
    this.ownerDocument = document;
  }
  getClientRects() {
    return this.visible ? [{}] : [];
  }
  closest() {
    return this.inert ? {} : null;
  }
  matches() {
    return this.disabled;
  }
  focus() {
    this.ownerDocument.activeElement = this;
  }
  contains(node: unknown) {
    return node === this;
  }
  querySelector() {
    return null;
  }
}
it('uses a persistent fallback when an opener disappears and preserves it across ref replay', () => {
  vi.stubGlobal('HTMLElement', FocusTarget);
  const document = { activeElement: null as FocusTarget | null };
  const opener = new FocusTarget(document);
  const surface = new FocusTarget(document);
  const fallback = new FocusTarget(document);
  document.activeElement = opener;
  const focus = useLayerFocus({ focusFallback: () => fallback as unknown as HTMLElement });
  focus.captureOrigin(surface as unknown as HTMLElement);
  document.activeElement = surface;
  focus.captureOrigin(surface as unknown as HTMLElement);
  opener.isConnected = false;
  const close = cancelable('autofocus');
  focus.onCloseAutoFocus(close);
  expect(document.activeElement).toBe(fallback);
  expect(close.defaultPrevented).toBe(true);
});
it('never focuses disconnected, invisible, inert or disabled return targets', () => {
  const document = { activeElement: null as FocusTarget | null };
  for (const property of ['isConnected', 'visible', 'inert', 'disabled'] as const) {
    const target = new FocusTarget(document);
    target[property] = property === 'inert' || property === 'disabled';
    expect(focusLayerTarget(target as unknown as HTMLElement)).toBe(false);
    expect(document.activeElement).toBeNull();
  }
});

describe('mounted surface viewport lifetime', () => {
  it('cleans up replay/unmount/repeated-open listeners and updates offset soft-keyboard bounds', () => {
    const viewport = Object.assign(new EventTarget(), {
      offsetLeft: 15,
      offsetTop: 30,
      width: 320,
      height: 390,
    });
    const view = Object.assign(new EventTarget(), {
      visualViewport: viewport,
      innerWidth: 1440,
      innerHeight: 900,
      getComputedStyle: () => ({ getPropertyValue: () => '4px' }),
    });
    const node = { ownerDocument: { defaultView: view } } as unknown as HTMLElement;
    const update = vi.fn();
    for (let cycle = 0; cycle < 3; cycle++) {
      const cleanup = observeLayerViewport(node, update);
      expect(update.mock.lastCall?.[0]).toMatchObject({
        '--tmt-layer-left': '15px',
        '--tmt-layer-top': '30px',
        '--tmt-layer-available-width': '304px',
        '--tmt-layer-available-height': '374px',
      });
      const before = update.mock.calls.length;
      viewport.height = 240;
      viewport.dispatchEvent(new Event('resize'));
      expect(update).toHaveBeenCalledTimes(before + 1);
      expect(update.mock.lastCall?.[0]['--tmt-layer-available-height']).toBe('224px');
      cleanup();
      cleanup();
      const after = update.mock.calls.length;
      for (const target of [view, viewport])
        for (const type of ['resize', 'scroll']) target.dispatchEvent(new Event(type));
      expect(update).toHaveBeenCalledTimes(after);
      viewport.height = 390;
    }
  });
});
it('keeps all layer class sources under the existing prefix/token policy', () => {
  const source = ['dialog.tsx', 'sheet.tsx', 'dropdown-menu.tsx', 'tooltip.tsx', 'popover.tsx']
    .map((file) => readFileSync(new URL(`../src/ui/${file}`, import.meta.url), 'utf8'))
    .join('\n');
  expect(uiCandidates(source).length).toBeGreaterThan(0);
  expect(source).not.toContain('document.body');
  expect(source).not.toMatch(/setTimeout|requestAnimationFrame/);
});
