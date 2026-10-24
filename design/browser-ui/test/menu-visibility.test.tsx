import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { createMenuController } from '../src/menu-controller.js';

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

function fixture(document?: object, start = true, replaceSelection = false) {
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
    ownerDocument,
    isConnected: true,
    open: false,
    style: {},
    dataset: {},
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
  const transitions: boolean[] = [];
  const selections: unknown[] = [];
  const replacementSelections: unknown[] = [];
  const event = {
    isTrusted: true,
    target: node as unknown as EventTarget,
    currentTarget: node,
    key: '',
    defaultPrevented: false,
    clientX: -1,
    clientY: -1,
    preventDefault() {},
    stopPropagation() {},
  };
  let select = (_key: string, selected: typeof event) => selections.push(selected);
  const controller = createMenuController({
    items: () => [{ key: 'one', label: 'One' }],
    disabled: () => false,
    hasDescription: () => false,
    trigger: () => anchor as unknown as HTMLButtonElement,
    surface: () => node as unknown as HTMLDialogElement,
    entries: () => [{ focus() {} } as HTMLButtonElement],
    selection: () => select,
    onOpenChange: (open) => {
      transitions.push(open);
      if (!open && replaceSelection) select = (_key, selected) => replacementSelections.push(selected);
    },
  });
  const mount = controller.attachSurface;
  if (start) controller.open();
  const cleanup = start ? mount()! : () => {};
  return {
    transitions,
    selections,
    replacementSelections,
    select: (selected: typeof event) => controller.select('one', selected),
    props: {
      onKeyDown: controller.menuKey,
      onClick: controller.surfaceClick,
      onClose: controller.nativeClose,
      onCancel: controller.cancel,
    },
    mount,
    cleanup,
    event,
    node,
    anchor,
    callbacks,
    ownerDocument,
    controller,
  };
}

describe('Menu visibility owner', () => {
  it('does not report an initial false while closed', () => {
    const f = fixture(undefined, false);
    expect(f.mount()).toBeUndefined();
    expect(f.transitions).toEqual([]);
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

it('captures the selected callback before a close notification replaces the host', () => {
  const f = fixture(undefined, true, true);
  f.select(f.event);
  expect(f.selections).toEqual([f.event]);
  expect(f.replacementSelections).toEqual([]);
});

it('closed and disposed menus dispatch no selection and cannot reopen after disposal', () => {
  const f = fixture();
  f.controller.close();
  f.select(f.event);
  expect(f.selections).toEqual([]);
  f.controller.dispose();
  f.controller.open();
  expect(f.controller.getSnapshot()).toEqual({ open: false, present: false });
  expect(f.transitions).toEqual([true, false]);
});
