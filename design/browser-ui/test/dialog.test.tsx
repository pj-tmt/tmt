import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserDialog } from '../src/dialog.js';
import { useModalDialog } from '../src/modal-dialog.js';
import { restoreOverlayFocus } from '../src/overlay.js';

const hooks = vi.hoisted(() => ({
  refs: [] as { current: unknown }[],
  index: 0,
  effect: undefined as (() => (() => void) | undefined) | undefined,
}));
vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  useId: () => 'fixture-dialog-title',
  useRef: (value: unknown) => {
    const index = hooks.index++;
    return (hooks.refs[index] ??= { current: value });
  },
  useLayoutEffect: (effect: () => (() => void) | undefined) => {
    hooks.effect = effect;
  },
}));
vi.mock('../src/overlay.js', () => ({
  restoreOverlayFocus: vi.fn(),
  outsideOverlay: () => true,
  admitsOverlayEscape: (event: {
    isTrusted: boolean;
    repeat?: boolean;
    nativeEvent?: { isComposing?: boolean };
  }) => event.isTrusted && !event.repeat && !event.nativeEvent?.isComposing,
}));
afterEach(() => {
  hooks.refs = [];
  hooks.index = 0;
  vi.clearAllMocks();
});

describe('content-fitting controlled Dialog', () => {
  it('associates its title, keeps host content and names its Close action', () => {
    const html = renderToStaticMarkup(
      <BrowserDialog open title="Search pages" onDismiss={() => {}}>
        <input aria-label="Search pages" />
      </BrowserDialog>,
    );
    expect(html).toContain('<dialog');
    expect(html).toContain('aria-labelledby="fixture-dialog-title"');
    expect(html).toContain('<h2 id="fixture-dialog-title">Search pages</h2>');
    expect(html).toContain('aria-label="Close Search pages"');
    expect(html).toContain('aria-label="Search pages"');
    expect(html).not.toContain('role="menu"');
  });
  it('has no surface while closed unless the host retains it', () => {
    expect(
      BrowserDialog({ open: false, title: 'Search', onDismiss() {}, children: 'Draft' }),
    ).toBeNull();
    hooks.index = 0;
    expect(
      renderToStaticMarkup(
        <BrowserDialog open={false} keepMounted title="Search" onDismiss={() => {}}>
          Draft
        </BrowserDialog>,
      ),
    ).toContain('Draft');
  });
});

function fixture(restoreFocus = true) {
  const origin = {};
  const target = { focus: vi.fn() };
  const node = {
    open: false,
    ownerDocument: { activeElement: origin },
    contains: (value: unknown) => value === target,
    showModal: vi.fn(function (this: { open: boolean }) {
      this.open = true;
    }),
    close: vi.fn(function (this: { open: boolean }) {
      this.open = false;
    }),
  };
  hooks.refs = [{ current: node }];
  const dismissed = vi.fn();
  const fallback = () => null;
  const props = {
    open: true,
    onDismiss: dismissed,
    initialFocus: () => target as unknown as HTMLElement,
    focusFallback: fallback,
    restoreFocus,
  };
  const modal = useModalDialog(props);
  const cleanup = hooks.effect!()!;
  const event = {
    target: node,
    currentTarget: node,
    isTrusted: true,
    key: 'Escape',
    defaultPrevented: false,
    nativeEvent: { isComposing: false },
    clientX: 0,
    clientY: 0,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  };
  return { node, target, origin, fallback, props, modal, dismissed, cleanup, event };
}

describe('shared native modal lifetime', () => {
  it('opens once, focuses the host target and restores its opener at cleanup', () => {
    const f = fixture();
    expect(f.node.showModal).toHaveBeenCalledOnce();
    expect(f.target.focus).toHaveBeenCalledWith({ preventScroll: true });
    f.cleanup();
    expect(f.node.close).toHaveBeenCalledOnce();
    expect(restoreOverlayFocus).toHaveBeenCalledWith(f.origin, f.fallback);
  });
  it('lets destination navigation take focus and does not dismiss for its own controlled close', () => {
    const f = fixture(false);
    hooks.index = 0;
    useModalDialog({ ...f.props, open: false });
    f.cleanup();
    f.modal.onClose(f.event as unknown as Parameters<typeof f.modal.onClose>[0]);
    expect(restoreOverlayFocus).not.toHaveBeenCalled();
    expect(f.dismissed).not.toHaveBeenCalled();
  });
  it('syncs a native close and preserves original events without consuming a child Escape', () => {
    const f = fixture();
    f.node.open = false;
    f.modal.onClose(f.event as unknown as Parameters<typeof f.modal.onClose>[0]);
    expect(f.dismissed).toHaveBeenCalledWith('close', f.event);
    f.dismissed.mockClear();
    f.modal.onKeyDown({ ...f.event, defaultPrevented: true } as unknown as Parameters<
      typeof f.modal.onKeyDown
    >[0]);
    f.modal.onKeyDown({ ...f.event, repeat: true } as unknown as Parameters<
      typeof f.modal.onKeyDown
    >[0]);
    f.modal.onKeyDown({ ...f.event, nativeEvent: { isComposing: true } } as unknown as Parameters<
      typeof f.modal.onKeyDown
    >[0]);
    expect(f.dismissed).not.toHaveBeenCalled();
    f.modal.onKeyDown(f.event as unknown as Parameters<typeof f.modal.onKeyDown>[0]);
    expect(f.dismissed).toHaveBeenCalledWith('escape', f.event);
    f.modal.onClick(f.event as unknown as Parameters<typeof f.modal.onClick>[0]);
    expect(f.dismissed).toHaveBeenCalledWith('backdrop', f.event);
    expect(f.event.preventDefault).toHaveBeenCalled();
  });
});
