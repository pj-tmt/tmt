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
  useImperativeHandle: vi.fn(),
  useEffect: (effect: () => (() => void) | undefined) => {
    hooks.effect = effect;
  },
}));
// The hook mocks above are positional for the tooltip; the busy mark has its own tests.
vi.mock('../src/busy-mark.js', () => ({ useBusyMark: () => false }));

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

it('dismisses an open tooltip without consuming the host Escape and releases its listener', () => {
  const events = new EventTarget();
  // Node's EventTarget removal ignores boolean capture options; adapt the DOM signature.
  const frames: (() => void)[] = [];
  const document = {
    defaultView: { requestAnimationFrame: (frame: () => void) => frames.push(frame) },
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
    expect(escape.defaultPrevented).toBe(false);
    expect(stop).not.toHaveBeenCalled();
    expect(dismiss).toHaveBeenCalledWith(true);
    expect(visible).toBe(false); // Native dismissal happens before the host handles Escape.
    // A tooltip can be hidden by native popover handling before React cleans up its effect.
    visible = false;
    const next = Object.assign(new Event('keydown', { cancelable: true }), { key: 'Escape' });
    const nextStop = vi.spyOn(next, 'stopPropagation');
    events.dispatchEvent(next);
    expect(next.defaultPrevented).toBe(false);
    expect(nextStop).not.toHaveBeenCalled();
    expect(dismiss).toHaveBeenCalledOnce();
    frames.splice(0).forEach((frame) => frame());
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

it.each([false, true])(
  'opens only on movement and keeps dismissal until leaving (menu=%s)',
  (menu) => {
    const hover = vi.fn();
    const dismiss = vi.fn();
    const render = (hovered: boolean, dismissed: boolean) => {
      hooks.refs = [{ current: null }, { current: null }];
      hooks.states = [
        [hovered, hover],
        [false, vi.fn()],
        [dismissed, dismiss],
      ];
      const props = {
        type: 'button' as const,
        label: 'Details',
        variant: 'text' as const,
        icon: '+',
        onActivate: vi.fn(),
      };
      const action = menu
        ? BrowserIconAction({ ...props, expanded: false, controls: 'host-menu', hasPopup: 'menu' })
        : BrowserIconAction(props);
      return action.props.children[0].props;
    };
    // A layout-only entry has no opening handler for either action variant.
    const initial = render(false, false);
    expect(initial.onPointerEnter).toBeUndefined();
    expect(hooks.effect!()).toBeUndefined();
    expect(hover).not.toHaveBeenCalled();
    initial.onPointerMove();
    expect(hover).toHaveBeenLastCalledWith(true);
    expect(dismiss).toHaveBeenLastCalledWith(false);
    hover.mockClear();
    dismiss.mockClear();
    // Click/Escape dismissal survives movement inside the same hovered action.
    const dismissed = render(true, true);
    dismissed.onPointerMove();
    expect(hover).not.toHaveBeenCalled();
    expect(dismiss).not.toHaveBeenCalled();
    dismissed.onPointerLeave();
    expect(hover).toHaveBeenLastCalledWith(false);
    render(false, true).onPointerMove();
    expect(dismiss).toHaveBeenLastCalledWith(false);
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
