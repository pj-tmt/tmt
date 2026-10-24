import { afterEach, expect, it, vi } from 'vite-plus/test';
import * as placement from '../src/icon-action-tooltip.js';
import { iconActionTooltipPosition } from '../src/icon-action-tooltip.js';
import { bindIconActionTooltip } from '../src/icon-action-tooltip.js';
afterEach(() => vi.unstubAllGlobals());

it('centers below the action and clamps long labels and both viewport edges', () => {
  const viewport = { left: 0, top: 0, width: 390, height: 844 };
  expect(
    iconActionTooltipPosition({ left: 180, right: 214, top: 16, bottom: 50 }, 100, 40, viewport, 8),
  ).toEqual({ left: 147, top: 50, maxWidth: 374, maxHeight: 786 });
  expect(
    iconActionTooltipPosition({ left: 0, right: 34, top: 16, bottom: 50 }, 100, 40, viewport, 8)
      .left,
  ).toBe(8);
  expect(
    iconActionTooltipPosition({ left: 356, right: 390, top: 16, bottom: 50 }, 100, 40, viewport, 8)
      .left,
  ).toBe(282);
  expect(
    iconActionTooltipPosition({ left: 356, right: 390, top: 16, bottom: 50 }, 800, 40, viewport, 8)
      .left,
  ).toBe(8);
});

it('flips above a bottom action within the offset visual viewport', () => {
  const viewport = { left: 20, top: 100, width: 320, height: 400 };
  expect(
    iconActionTooltipPosition({ left: 20, right: 54, top: 446, bottom: 480 }, 400, 40, viewport, 8),
  ).toEqual({ left: 28, top: 406, maxWidth: 304, maxHeight: 338 });
});

it('clamps to the larger side when neither side fits and prefers below on a tie', () => {
  const viewport = { left: 20, top: 100, width: 320, height: 400 };
  expect(
    iconActionTooltipPosition(
      { left: 20, right: 54, top: 330, bottom: 364 },
      100,
      500,
      viewport,
      8,
    ),
  ).toEqual({ left: 28, top: 108, maxWidth: 304, maxHeight: 222 });
  expect(
    iconActionTooltipPosition(
      { left: 20, right: 54, top: 200, bottom: 234 },
      100,
      500,
      viewport,
      8,
    ),
  ).toEqual({ left: 28, top: 234, maxWidth: 304, maxHeight: 258 });
  expect(
    iconActionTooltipPosition(
      { left: 20, right: 54, top: 283, bottom: 317 },
      100,
      500,
      viewport,
      8,
    ),
  ).toEqual({ left: 28, top: 317, maxWidth: 304, maxHeight: 175 });
});

function bindingFixture(expanded?: boolean) {
  const events = new EventTarget();
  const frames: (() => void)[] = [];
  const disconnects: ReturnType<typeof vi.fn>[] = [];
  vi.stubGlobal(
    'ResizeObserver',
    class {
      disconnect = vi.fn();
      constructor() {
        disconnects.push(this.disconnect);
      }
      observe() {}
    },
  );
  const add = vi.fn((type: string, listener: EventListener, capture: boolean) =>
    events.addEventListener(type, listener, { capture }),
  );
  const document = {
    defaultView: {
      innerWidth: 390,
      innerHeight: 844,
      requestAnimationFrame: (frame: () => void) => frames.push(frame),
      cancelAnimationFrame() {},
      getComputedStyle: () => ({ paddingTop: '8px' }),
      addEventListener() {},
      removeEventListener() {},
    },
    addEventListener: add,
    removeEventListener(type: string, listener: EventListener, capture: boolean) {
      events.removeEventListener(type, listener, { capture });
    },
  };
  const state = { visible: false, expanded, focusVisible: false };
  const wrapper = new EventTarget();
  const button = Object.assign(new EventTarget(), {
    ownerDocument: document,
    isConnected: true,
    getClientRects: () => [1],
    getAttribute: () => (state.expanded === undefined ? null : String(state.expanded)),
    matches: () => state.focusVisible,
    getBoundingClientRect: () => ({
      left: 180,
      right: 214,
      top: 16,
      bottom: 50,
      width: 34,
      height: 34,
    }),
  });
  const show = vi.fn(() => {
    state.visible = true;
  });
  const hide = vi.fn(() => {
    state.visible = false;
  });
  const tooltip = {
    style: {},
    scrollHeight: 40,
    offsetHeight: 40,
    clientHeight: 40,
    showPopover: show,
    hidePopover: hide,
    matches: () => state.visible,
    getBoundingClientRect: () => ({ width: 100, height: 40 }),
  };
  const binding = bindIconActionTooltip(
    wrapper as HTMLElement,
    button as unknown as HTMLButtonElement,
    tooltip as unknown as HTMLElement,
  );
  return { binding, wrapper, button, state, show, hide, events, frames, disconnects, add };
}

it('dismisses an open tooltip without consuming the host Escape and releases its listener', () => {
  const f = bindingFixture();
  f.wrapper.dispatchEvent(new Event('pointermove'));
  expect(f.state.visible).toBe(true);
  const escape = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
  const stop = vi.spyOn(escape, 'stopPropagation');
  f.events.dispatchEvent(escape);
  expect(escape.defaultPrevented).toBe(false);
  expect(stop).not.toHaveBeenCalled();
  expect(f.state.visible).toBe(false);
  expect(f.hide).toHaveBeenCalledOnce();
  const next = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
  const nextStop = vi.spyOn(next, 'stopPropagation');
  f.events.dispatchEvent(next);
  expect(next.defaultPrevented).toBe(false);
  expect(nextStop).not.toHaveBeenCalled();
  expect(f.hide).toHaveBeenCalledOnce();
  f.frames.splice(0).forEach((frame) => frame());
  f.state.visible = true;
  f.binding.dispose();
  expect(f.state.visible).toBe(false);
  expect(f.disconnects.every((disconnect) => disconnect.mock.calls.length === 1)).toBe(true);
  f.state.visible = true;
  const removed = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
  f.events.dispatchEvent(removed);
  expect(removed.defaultPrevented).toBe(false);
  expect(f.hide).toHaveBeenCalledTimes(2);
});

it('leaves tooltip placement and Escape with the host while disclosure is expanded', () => {
  const f = bindingFixture(true);
  f.wrapper.dispatchEvent(new Event('pointermove'));
  expect(f.show).not.toHaveBeenCalled();
  expect(f.disconnects).toHaveLength(0);
  expect(f.add).not.toHaveBeenCalled();
  f.binding.dispose();
});

it.each([false, true])(
  'opens only on movement and keeps dismissal until leaving (menu=%s)',
  (menu) => {
    const f = bindingFixture(menu ? false : undefined);
    f.wrapper.dispatchEvent(new Event('pointerenter'));
    expect(f.show).not.toHaveBeenCalled();
    f.wrapper.dispatchEvent(new Event('pointermove'));
    expect(f.state.visible).toBe(true);
    f.binding.dismiss();
    f.wrapper.dispatchEvent(new Event('pointermove'));
    expect(f.state.visible).toBe(false);
    expect(f.show).toHaveBeenCalledOnce();
    f.wrapper.dispatchEvent(new Event('pointerleave'));
    f.wrapper.dispatchEvent(new Event('pointermove'));
    expect(f.state.visible).toBe(true);
    expect(f.show).toHaveBeenCalledTimes(2);
    f.binding.dispose();
  },
);

it('replaces help per document and stale cleanup cannot release the next tooltip', () => {
  const document = {} as Document;
  const otherDocument = {} as Document;
  const first = vi.fn();
  const second = vi.fn();
  const independent = vi.fn();
  const releaseFirst = placement.claimIconActionTooltip(document, first);
  const releaseIndependent = placement.claimIconActionTooltip(otherDocument, independent);
  expect(first).not.toHaveBeenCalled();
  const releaseSecond = placement.claimIconActionTooltip(document, second);
  expect(first).toHaveBeenCalledTimes(1);
  expect(independent).not.toHaveBeenCalled();
  releaseFirst();
  const third = vi.fn();
  const releaseThird = placement.claimIconActionTooltip(document, third);
  expect(second).toHaveBeenCalledTimes(1);
  releaseSecond();
  releaseThird();
  const releaseAfterUnmount = placement.claimIconActionTooltip(document, vi.fn());
  expect(third).not.toHaveBeenCalled();
  releaseAfterUnmount();
  releaseIndependent();
});
