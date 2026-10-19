import { renderToStaticMarkup } from 'react-dom/server';
import type { MouseEvent, ReactElement } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vite-plus/test';
import { BrowserAction, BrowserIconAction } from '../src/react';
import { busyMarkHoldMs, busyMarkDelayMs, createBusyMarkGate } from '../src/busy-mark';

let shown: boolean[];
let gate: ReturnType<typeof createBusyMarkGate>;
beforeEach(() => {
  vi.useFakeTimers();
  shown = [];
  gate = createBusyMarkGate((value) => shown.push(value));
});
afterEach(() => {
  gate.dispose();
  vi.useRealTimers();
});

it('uses the decided 400 ms delay and 400 ms minimum', () => {
  expect([busyMarkDelayMs, busyMarkHoldMs]).toEqual([400, 400]);
});

it('shows no mark for busy shorter than the delay', () => {
  gate.set(true);
  vi.advanceTimersByTime(399);
  gate.set(false);
  vi.advanceTimersByTime(2000);
  expect(shown).toEqual([]);
  expect(gate.shown()).toBe(false);
});

it('shows the mark from 400 ms and keeps it 400 ms after a busy that ends just after', () => {
  gate.set(true);
  vi.advanceTimersByTime(399);
  expect(shown).toEqual([]);
  vi.advanceTimersByTime(1);
  expect(shown).toEqual([true]);
  vi.advanceTimersByTime(10);
  gate.set(false);
  vi.advanceTimersByTime(389);
  expect(shown).toEqual([true]);
  vi.advanceTimersByTime(1);
  expect(shown).toEqual([true, false]);
});

it('hides at once when a long busy ends after the minimum', () => {
  gate.set(true);
  vi.advanceTimersByTime(1500);
  gate.set(false);
  expect(shown).toEqual([true, false]);
});

it('keeps one mark when busy resumes during the minimum hold', () => {
  gate.set(true);
  vi.advanceTimersByTime(450);
  gate.set(false);
  vi.advanceTimersByTime(100);
  gate.set(true);
  vi.advanceTimersByTime(2000);
  expect(shown).toEqual([true]);
  gate.set(false);
  expect(shown).toEqual([true, false]);
});

it('restarts the delay for a new busy after a short one', () => {
  gate.set(true);
  vi.advanceTimersByTime(300);
  gate.set(false);
  gate.set(true);
  vi.advanceTimersByTime(399);
  expect(shown).toEqual([]);
  vi.advanceTimersByTime(1);
  expect(shown).toEqual([true]);
});

it('emits nothing after dispose', () => {
  gate.set(true);
  gate.dispose();
  vi.advanceTimersByTime(2000);
  expect(shown).toEqual([]);
});

const button = (element: ReactElement) =>
  (element.props as { children: ReactElement[] }).children[0] as ReactElement<{
    onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  }>;

it('keeps a busy action focusable and readable: aria-disabled, not disabled', () => {
  for (const html of [
    renderToStaticMarkup(
      <BrowserAction
        type="submit"
        label="Save"
        variant="primary"
        busy
        busyMark="◌"
        onActivate={() => {}}
      />,
    ),
    renderToStaticMarkup(
      <BrowserIconAction
        type="button"
        label="Check"
        variant="text"
        icon="+"
        busy
        busyMark="◌"
        onActivate={() => {}}
      />,
    ),
  ]) {
    expect(html).not.toContain('disabled=""');
    expect(html).toContain('aria-disabled="true"');
    expect(html).toContain('aria-busy="true"');
    // The mark is decided by the delay gate, so a busy first frame shows none.
    expect(html).toContain('data-busy="false"');
  }
});

it('keeps real disabled for an unavailable action, busy or not', () => {
  for (const busy of [false, true]) {
    const html = renderToStaticMarkup(
      <BrowserAction
        type="button"
        label="Save"
        variant="primary"
        disabled
        busy={busy}
        onActivate={() => {}}
      />,
    );
    expect(html).toContain('disabled=""');
    expect(html).not.toContain('aria-disabled');
  }
});

it('guards busy activation and cancels its default form submission', () => {
  const calls: unknown[] = [];
  const preventDefault = vi.fn();
  const event = {
    preventDefault,
    nativeEvent: { type: 'click', detail: 0 },
  } as unknown as MouseEvent<HTMLButtonElement>;
  // Render the component functions directly; the hooks only need a dispatcher for state.
  const html = renderToStaticMarkup(
    <BusyProbe onClick={(click) => click(event)} onActivate={(value) => calls.push(value)} />,
  );
  expect(html).toBe('');
  expect(calls).toEqual([]);
  expect(preventDefault).toHaveBeenCalledTimes(2);
});

function BusyProbe({
  onClick,
  onActivate,
}: {
  onClick: (click: (event: MouseEvent<HTMLButtonElement>) => void) => void;
  onActivate: (event: MouseEvent<HTMLButtonElement>) => void;
}) {
  onClick(
    button(
      BrowserAction({ type: 'submit', label: 'Save', variant: 'primary', busy: true, onActivate }),
    ).props.onClick,
  );
  const icon = BrowserIconAction({
    type: 'submit',
    label: 'Check',
    variant: 'text',
    icon: '+',
    busy: true,
    onActivate,
  });
  const iconButton = (
    (icon.props as { children: ReactElement[] }).children[0].props as { children: ReactElement[] }
  ).children[0] as ReactElement<{ onClick: (event: MouseEvent<HTMLButtonElement>) => void }>;
  onClick(iconButton.props.onClick);
  return null;
}
