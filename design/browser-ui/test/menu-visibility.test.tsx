import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import type { ReactElement } from 'react';
import { BrowserMenu } from '../src/menu.js';

const hooks = vi.hoisted(() => ({
  refs: [] as { current: unknown }[],
  index: 0,
  open: true,
  effect: undefined as (() => (() => void) | undefined) | undefined,
}));
vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  useId: () => 'fixture-menu',
  useRef: (value: unknown) => hooks.refs[hooks.index++] ?? { current: value },
  useState: () => [hooks.open, vi.fn()],
  useLayoutEffect: (effect: () => (() => void) | undefined) => {
    hooks.effect = effect;
  },
}));
vi.mock('../src/overlay.js', () => ({
  restoreOverlayFocus: vi.fn(),
  outsideOverlay: () => true,
  admitsOverlayEscape: (event: { isTrusted: boolean; shiftKey?: boolean }) =>
    event.isTrusted && !event.shiftKey,
}));
vi.mock('../src/menu-position.js', () => ({
  browserMenuPosition: () => ({ left: 0, top: 0, maxHeight: 300 }),
}));
afterEach(() => vi.unstubAllGlobals());

function fixture(document?: object) {
  const callbacks: (() => void)[] = [];
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(callback: () => void) {
        callbacks.push(callback);
      }
      observe() {}
      disconnect() {}
    },
  );
  const ownerDocument = document ?? {
    defaultView: {
      requestAnimationFrame(callback: FrameRequestCallback) {
        callback(0);
        return 0;
      },
      cancelAnimationFrame() {},
      innerWidth: 1440,
      innerHeight: 900,
      addEventListener() {},
      removeEventListener() {},
    },
  };
  const anchor = {
    ownerDocument,
    isConnected: true,
    getClientRects: () => [1],
    getBoundingClientRect: () => ({ left: 0, top: 0, right: 40, bottom: 30 }),
  };
  const node = {
    isConnected: true,
    open: false,
    style: {},
    scrollHeight: 40,
    offsetHeight: 40,
    clientHeight: 40,
    showModal() {
      this.open = true;
    },
    close() {
      this.open = false;
    },
    focus() {},
    getBoundingClientRect: () => ({ width: 100, height: 40 }),
  };
  hooks.index = 0;
  hooks.open = true;
  hooks.refs = [{ current: anchor }, { current: node }, { current: [{ focus() {} }] }];
  const transitions: boolean[] = [];
  const selections: unknown[] = [];
  const element = BrowserMenu({
    label: 'Fixture',
    items: [{ key: 'one', label: 'One' }],
    onSelect: (_key, event) => selections.push(event),
    onOpenChange: (open) => transitions.push(open),
  });
  const dialog = (element.props as { children: ReactElement[] }).children[1];
  const props = dialog.props as Record<string, (event: unknown) => void>;
  const menu = (dialog.props as { children: ReactElement }).children;
  const rows = (menu.props as { children: [unknown, ReactElement[]] }).children[1];
  const button = (rows[0].props as { children: ReactElement[] }).children[0];
  const select = (button.props as { onClick(event: unknown): void }).onClick;
  const mount = hooks.effect!;
  const cleanup = mount()!;
  const event = {
    isTrusted: true,
    target: node,
    currentTarget: node,
    key: '',
    preventDefault() {},
    stopPropagation() {},
  };
  return {
    transitions,
    selections,
    select,
    props,
    mount,
    cleanup,
    event,
    node,
    anchor,
    callbacks,
    ownerDocument,
  };
}

describe('Menu visibility owner', () => {
  it('does not report an initial false while closed', () => {
    hooks.index = 0;
    hooks.refs = [];
    hooks.open = false;
    const transitions: boolean[] = [];
    BrowserMenu({
      label: 'Closed',
      items: [{ key: 'one', label: 'One' }],
      onSelect() {},
      onOpenChange: (value) => transitions.push(value),
    });
    expect(hooks.effect!()).toBeUndefined();
    expect(transitions).toEqual([]);
  });
  it('reports exactly one true after native opening', () => {
    const f = fixture();
    expect(f.transitions).toEqual([true]);
    f.cleanup();
  });
  for (const path of [
    'selection',
    'Escape',
    'Tab',
    'backdrop',
    'native close',
    'cancel',
    'anchor removal',
  ] as const) {
    it(`reports exactly one close for ${path}, even when the native close event follows`, () => {
      const f = fixture();
      if (path === 'selection') f.select(f.event);
      else if (path === 'Escape' || path === 'Tab')
        f.props.onKeyDown({ ...f.event, key: path, nativeEvent: {} });
      else if (path === 'backdrop') f.props.onClick(f.event);
      else if (path === 'native close') {
        f.node.open = false;
        f.props.onClose(f.event);
      } else if (path === 'cancel') f.props.onCancel(f.event);
      else {
        f.anchor.isConnected = false;
        f.callbacks[0]();
      }
      f.props.onClose(f.event);
      f.cleanup();
      expect(f.transitions).toEqual([true, false]);
      if (path === 'selection') expect(f.selections[0]).toBe(f.event);
    });
  }
  it('reports replacement before the next menu opens', () => {
    const first = fixture();
    const second = fixture(first.ownerDocument);
    expect(first.transitions).toEqual([true, false]);
    expect(second.transitions).toEqual([true]);
    first.cleanup();
    second.cleanup();
  });
  it('does not report StrictMode cleanup/replay as a close, but reports actual unmount once', async () => {
    const f = fixture();
    f.cleanup();
    const finalCleanup = f.mount()!;
    await Promise.resolve();
    expect(f.transitions).toEqual([true]);
    f.node.isConnected = false;
    finalCleanup();
    await Promise.resolve();
    expect(f.transitions).toEqual([true, false]);
  });
  it('does not dismiss or report modified or untrusted Escape', () => {
    const f = fixture();
    f.props.onKeyDown({ ...f.event, key: 'Escape', shiftKey: true });
    f.props.onKeyDown({ ...f.event, key: 'Escape', isTrusted: false });
    expect(f.transitions).toEqual([true]);
    f.cleanup();
  });
});
