import { expect, it, vi } from 'vite-plus/test';
import * as placement from '../src/icon-action-tooltip.js';
import { iconActionTooltipPosition } from '../src/icon-action-tooltip.js';
import { BrowserIconAction } from '../src/icon-action.js';

const hooks = vi.hoisted(() => ({
  refs: [] as { current: unknown }[],
  states: [] as [boolean, (value: boolean) => void][],
  effect: undefined as (() => (() => void) | undefined) | undefined,
}));
vi.mock('react', async (importOriginal) => ({
  ...(await importOriginal<typeof import('react')>()),
  useRef: () => hooks.refs.shift(),
  useState: () => hooks.states.shift(),
  useEffect: (effect: () => (() => void) | undefined) => {
    hooks.effect = effect;
  },
}));

it('centers below the action and clamps long labels and both viewport edges', () => {
  const viewport = { left: 0, top: 0, width: 390, height: 844 };
  expect(
    iconActionTooltipPosition({ left: 180, right: 214, bottom: 50 }, 100, viewport, 8),
  ).toEqual({ left: 147, top: 50, maxWidth: 374, maxHeight: 786 });
  expect(iconActionTooltipPosition({ left: 0, right: 34, bottom: 50 }, 100, viewport, 8).left).toBe(
    8,
  );
  expect(
    iconActionTooltipPosition({ left: 356, right: 390, bottom: 50 }, 100, viewport, 8).left,
  ).toBe(282);
  expect(
    iconActionTooltipPosition({ left: 356, right: 390, bottom: 50 }, 800, viewport, 8).left,
  ).toBe(8);
});

it('uses the visual viewport offset and bounds the remaining space without flipping above', () => {
  const viewport = { left: 20, top: 100, width: 320, height: 400 };
  expect(iconActionTooltipPosition({ left: 20, right: 54, bottom: 480 }, 400, viewport, 8)).toEqual(
    { left: 28, top: 480, maxWidth: 304, maxHeight: 12 },
  );
  expect(
    iconActionTooltipPosition({ left: 20, right: 54, bottom: 510 }, 40, viewport, 8).maxHeight,
  ).toBe(0);
});

it('consumes Escape only while the tooltip is visible and releases the listener on dismissal', () => {
  const events = new EventTarget();
  // Node's EventTarget removal ignores boolean capture options; adapt the DOM signature.
  const document = {
    addEventListener(type: string, listener: EventListener, capture: boolean) {
      events.addEventListener(type, listener, { capture });
    },
    removeEventListener(type: string, listener: EventListener, capture: boolean) {
      events.removeEventListener(type, listener, { capture });
    },
  };
  let visible = false;
  const dismiss = vi.fn();
  const tooltip = {
    showPopover: () => {
      visible = true;
    },
    hidePopover: () => {
      visible = false;
    },
    matches: () => visible,
  };
  hooks.refs = [{ current: { ownerDocument: document } }, { current: tooltip }];
  hooks.states = [
    [true, vi.fn()],
    [false, vi.fn()],
    [false, dismiss],
  ];
  const disposePlacement = vi.fn();
  const place = vi.spyOn(placement, 'placeIconActionTooltip').mockReturnValue(disposePlacement);
  try {
    BrowserIconAction({
      type: 'button',
      label: 'Details',
      variant: 'text',
      icon: '+',
      onActivate: vi.fn(),
    });
    const dispose = hooks.effect!()!;
    const escape = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
    const stop = vi.spyOn(escape, 'stopPropagation');
    events.dispatchEvent(escape);
    expect(escape.defaultPrevented).toBe(true);
    expect(stop).toHaveBeenCalledOnce();
    expect(dismiss).toHaveBeenCalledWith(true);
    // A tooltip can be hidden by native popover handling before React cleans up its effect.
    visible = false;
    const next = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
    const nextStop = vi.spyOn(next, 'stopPropagation');
    events.dispatchEvent(next);
    expect(next.defaultPrevented).toBe(false);
    expect(nextStop).not.toHaveBeenCalled();
    expect(dismiss).toHaveBeenCalledOnce();
    visible = true;
    dispose();
    expect(visible).toBe(false);
    expect(disposePlacement).toHaveBeenCalledOnce();
    visible = true;
    const removed = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
    events.dispatchEvent(removed);
    expect(removed.defaultPrevented).toBe(false);
    expect(dismiss).toHaveBeenCalledOnce();
  } finally {
    place.mockRestore();
  }
});

it('leaves tooltip placement and Escape with the host while disclosure is expanded', () => {
  const show = vi.fn();
  const add = vi.fn();
  hooks.refs = [
    { current: { ownerDocument: { addEventListener: add } } },
    { current: { showPopover: show } },
  ];
  hooks.states = [
    [true, vi.fn()],
    [true, vi.fn()],
    [false, vi.fn()],
  ];
  const place = vi.spyOn(placement, 'placeIconActionTooltip');
  try {
    BrowserIconAction({
      type: 'button',
      label: 'More actions',
      variant: 'text',
      icon: '+',
      expanded: true,
      controls: 'host-menu',
      onActivate: vi.fn(),
    });
    expect(hooks.effect!()).toBeUndefined();
    expect(show).not.toHaveBeenCalled();
    expect(place).not.toHaveBeenCalled();
    expect(add).not.toHaveBeenCalled();
  } finally {
    place.mockRestore();
  }
});
