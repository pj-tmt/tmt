import { act, createRef } from 'react';
import type { MouseEvent } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserAction, BrowserField, BrowserHeader, BrowserIconAction } from '../../src/react.js';
import { checkFieldFocus } from './field-focus.js';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
  true;
const host = document.querySelector<HTMLElement>('#root')!;
const root = createRoot(host);
const result = document.querySelector<HTMLElement>('#result')!;
let assertions = 0;
function check(value: unknown, description: string) {
  if (!value) throw new Error(description);
  assertions += 1;
}
function tokenColor(name: string) {
  const probe = document.createElement('span');
  probe.style.color = getComputedStyle(host).getPropertyValue(name);
  return probe.style.color;
}
let activations = 0;
let parentEscapes = 0;
let lastEvent: MouseEvent<HTMLButtonElement> | undefined;
document.body.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') parentEscapes += 1;
});
const onActivate = (event: MouseEvent<HTMLButtonElement>) => {
  activations += 1;
  lastEvent = event;
  result.dataset.activations = String(activations);
};

async function run() {
  assertions = 0;
  activations = 0;
  parentEscapes = 0;
  for (const linked of [false, true]) {
    await act(() =>
      root.render(
        <BrowserHeader
          productLabel="Colab"
          title="Pages"
          brandLink={linked ? (brand) => <a href="#pages">{brand}</a> : undefined}
        />,
      ),
    );
    const header = host.querySelector('header')!;
    const mark = header.querySelector<SVGSVGElement>('svg.tmt-ui-mark')!;
    check(mark !== null, 'Header mark is not an inline SVG');
    check(mark.getAttribute('aria-hidden') === 'true', 'Header mark is exposed to accessibility');
    check(mark.getAttribute('fill') === 'currentColor', 'Header mark has a fixed colour');
    const label = header.querySelector<HTMLElement>('.tmt-ui-wordmark')!;
    const brand = header.querySelector<HTMLElement>(
      linked ? '.tmt-ui-brand > a' : '.tmt-ui-brand',
    )!;
    const heading = header.querySelector<HTMLElement>('.tmt-ui-heading')!;
    const markBounds = mark.getBoundingClientRect();
    const headerBounds = header.getBoundingClientRect();
    const labelSize = parseFloat(getComputedStyle(label).fontSize);
    check(
      Math.abs(markBounds.width - (labelSize * 33) / 19) < 0.1 &&
        Math.abs(markBounds.height - markBounds.width) < 0.1 &&
        Math.abs(parseFloat(getComputedStyle(brand).gap) - (labelSize * 12) / 19) < 0.1,
      'Header brand lost the handbook proportions',
    );
    check(
      markBounds.top >= headerBounds.top && markBounds.bottom <= headerBounds.bottom,
      'Header mark exceeds header height',
    );
    check(
      getComputedStyle(label).color === tokenColor('--tmt-ui-color-text') &&
        getComputedStyle(mark).color === tokenColor('--tmt-ui-color-text'),
      'Header brand lost the normal text colour',
    );
    check(getComputedStyle(heading).borderLeftWidth === '0px', 'Header title retains a divider');
    check(header.querySelector('h1')!.textContent === 'Pages', 'Header title changed');
    if (linked) check(brand.textContent === 'Colab', 'Decorative mark contributes to link name');
  }
  await act(() =>
    root.render(
      <>
        <BrowserField
          controlId="focus-native"
          label="Native message"
          renderControl={(props) => <textarea {...props} defaultValue="Native draft" />}
        />
        <BrowserField
          controlId="focus-editable"
          label="Editable message"
          renderControl={(props) => (
            <div
              {...props}
              role="textbox"
              tabIndex={0}
              contentEditable
              suppressContentEditableWarning
            >
              Editable draft
            </div>
          )}
        />
      </>,
    ),
  );
  assertions += checkFieldFocus([
    document.getElementById('focus-native')!,
    document.getElementById('focus-editable')!,
  ]);
  const ref = createRef<HTMLDivElement>();
  let compositions = 0;
  const field = (readonly: boolean) => (
    <BrowserField
      controlId="live-message"
      label="Message"
      description={readonly ? 'Read only' : 'Draft kept'}
      descriptionId="live-access"
      invalid={readonly}
      error={readonly ? 'Access changed' : undefined}
      errorId="live-error"
      renderControl={(props) => (
        <div
          {...props}
          ref={ref}
          role="textbox"
          tabIndex={0}
          aria-multiline="true"
          aria-readonly={readonly}
          contentEditable={!readonly}
          suppressContentEditableWarning
          onCompositionStart={() => {
            compositions += 1;
          }}
          onCompositionEnd={() => {
            compositions += 1;
          }}
        >
          Original draft
        </div>
      )}
    />
  );
  await act(() => root.render(field(false)));
  const original = ref.current!;
  original.focus();
  const label = document.getElementById('live-message-label')!;
  const fieldStyle = getComputedStyle(original);
  check(
    original.getBoundingClientRect().top - label.getBoundingClientRect().bottom >=
      parseFloat(fieldStyle.outlineWidth) + parseFloat(fieldStyle.outlineOffset),
    'Field label overlaps the focus outline',
  );
  // Non-English text is deliberate composition/Unicode fixture data.
  original.textContent = 'Draft <literal> with 日本語';
  const text = original.firstChild!;
  const range = document.createRange();
  range.setStart(text, 6);
  range.collapse(true);
  const selection = window.getSelection()!;
  selection.removeAllRanges();
  selection.addRange(range);
  const compositionStart = new CompositionEvent('compositionstart', { bubbles: true, data: '語' });
  await act(() => original.dispatchEvent(compositionStart));
  await act(() => root.render(field(true)));
  check(ref.current === original, 'Access update remounted the editor or ref');
  check(
    original.textContent === 'Draft <literal> with 日本語',
    'Access update erased the host draft',
  );
  check(
    selection.anchorNode === text && selection.anchorOffset === 6,
    'Access update reset the caret',
  );
  check(original.getAttribute('aria-labelledby') === 'live-message-label', 'Missing editor name');
  check(original.getAttribute('aria-describedby') === 'live-access live-error', 'Stale ARIA IDs');
  await act(() =>
    original.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '語' })),
  );
  check(compositions === 2, 'Composition handlers did not retain the editor lifetime');
  await act(() => root.render(field(false)));
  check(
    ref.current === original && original.textContent === 'Draft <literal> with 日本語',
    'Restore reset the editor',
  );
  check(original.getAttribute('aria-describedby') === 'live-access', 'Absent error ID remained');

  const action = (state: { pressed?: boolean; disabled?: boolean; busy?: boolean } = {}) => (
    <BrowserIconAction
      type="button"
      label="Show details"
      variant="text"
      icon="+"
      busyMark="◌"
      onActivate={onActivate}
      {...state}
    />
  );
  await act(() => root.render(action({ pressed: true })));
  const button = host.querySelector('button')!;
  const tooltip = host.querySelector<HTMLElement>('[popover]')!;
  const click = new window.MouseEvent('click', { bubbles: true, detail: 0 });
  await act(() => button.dispatchEvent(click));
  check(
    lastEvent?.nativeEvent === click && activations === 1,
    'Activation stripped the original event',
  );
  check(button.getAttribute('aria-pressed') === 'true', 'Pressed state changed itself');
  check(
    getComputedStyle(button).backgroundColor === tokenColor('--tmt-ui-surface-selection'),
    'Pressed icon lost its selection fill',
  );
  for (const state of [{ busy: true }, { disabled: true }]) {
    await act(() => root.render(action(state)));
    await act(() => button.click());
    check(activations === 1, 'Busy/disabled action delivered an activation');
    const style = getComputedStyle(button);
    check(style.backgroundColor === 'rgba(0, 0, 0, 0)', 'Disabled text icon has a fill');
    check(style.borderTopColor === 'rgba(0, 0, 0, 0)', 'Disabled text icon has a visible edge');
    check(
      style.color === tokenColor('--tmt-ui-color-disabled-text'),
      'Disabled text icon lost its muted text',
    );
  }
  await act(() => root.render(action({ pressed: false })));
  check(host.querySelector('button') === button, 'Action update remounted the native button');
  check(button.getAttribute('aria-pressed') === 'false', 'Caller pressed update was ignored');
  check(
    getComputedStyle(button).backgroundColor === 'rgba(0, 0, 0, 0)',
    'Ready text icon has a fill',
  );
  await act(() => button.dispatchEvent(new PointerEvent('pointerover', { bubbles: true })));
  check(tooltip.matches(':popover-open'), 'Pointer entry did not show the tooltip');
  check(tooltip.getAttribute('aria-hidden') === 'true', 'Tooltip duplicates the accessible name');
  check(!button.hasAttribute('aria-describedby'), 'Label was announced twice');
  await act(() =>
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
  );
  check(
    !tooltip.matches(':popover-open') && parentEscapes === 1,
    'Escape must dismiss the tooltip and reach its host',
  );
  await act(() =>
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
  );
  check(parentEscapes === 2, 'Closed tooltip swallowed parent Escape');
  await act(() => button.dispatchEvent(new PointerEvent('pointerout', { bubbles: true })));
  await act(() => button.dispatchEvent(new PointerEvent('pointerover', { bubbles: true })));
  check(tooltip.matches(':popover-open'), 'Pointer return did not reopen the tooltip');
  await act(() => root.render(null));
  check(!tooltip.isConnected, 'Tooltip survived component removal');
  await act(() =>
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
  );
  check(parentEscapes === 3, 'Tooltip listener survived component removal');
  for (const variant of ['text', 'primary', 'destructive'] as const) {
    for (const busyMark of [
      undefined,
      '◌',
      <svg
        key="busy-mark"
        className="fixture-busy-mark"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
      >
        <circle cx="12" cy="12" r="8" />
      </svg>,
    ]) {
      for (const stretched of [false, true]) {
        const description = `${variant}, ${busyMark === undefined ? 'no mark' : typeof busyMark === 'string' ? 'text mark' : 'SVG mark'}, ${stretched ? 'stretched' : 'intrinsic'}`;
        const renderAction = (busy: boolean) => (
          <div className={stretched ? 'fixture-stretched-action' : undefined}>
            <BrowserAction
              type="button"
              label="Reconnect"
              variant={variant}
              busy={busy}
              busyMark={busyMark}
              onActivate={onActivate}
            />
          </div>
        );
        await act(() => root.render(renderAction(false)));
        const originalButton = host.querySelector('button')!;
        const readyBounds = originalButton.getBoundingClientRect();
        for (const busy of [false, true, false]) {
          await act(() => root.render(renderAction(busy)));
          const currentButton = host.querySelector('button')!;
          const bounds = currentButton.getBoundingClientRect();
          const labelBounds = currentButton
            .querySelector('.tmt-ui-action-label')!
            .getBoundingClientRect();
          check(currentButton === originalButton, `Action remounted: ${description}`);
          check(
            Math.abs(bounds.width - readyBounds.width) <= 1 &&
              Math.abs(bounds.height - readyBounds.height) <= 1,
            `Busy changed action geometry: ${description}`,
          );
          check(
            Math.abs(labelBounds.left - bounds.left - (bounds.right - labelBounds.right)) <= 1,
            `Action label is off center: ${description}, busy=${busy}`,
          );
          check(currentButton.disabled === busy, `Busy activation fence changed: ${description}`);
          const mark = currentButton.querySelector('.tmt-ui-action-mark');
          if (busyMark === undefined) {
            check(mark === null, `No-mark action reserved a mark: ${description}`);
            check(
              getComputedStyle(currentButton).display === 'inline-flex',
              `No-mark action lost label-only layout: ${description}`,
            );
          } else {
            check(
              mark?.getAttribute('aria-hidden') === 'true',
              `Mark lost decoration: ${description}`,
            );
            check(
              getComputedStyle(mark!).visibility === (busy ? 'visible' : 'hidden'),
              `Mark visibility disagrees with busy state: ${description}`,
            );
          }
          if (stretched)
            check(
              Math.abs(bounds.width - currentButton.parentElement!.getBoundingClientRect().width) <=
                1,
              `Host did not stretch the action: ${description}`,
            );
        }
      }
    }
  }
  for (const state of [{}, { disabled: true }, { busy: true }]) {
    await act(() =>
      root.render(
        <BrowserAction
          type="button"
          label="Change recipient"
          variant="text"
          onActivate={onActivate}
          {...state}
        />,
      ),
    );
    const textAction = host.querySelector('button')!;
    check(
      getComputedStyle(textAction).backgroundColor === 'rgba(0, 0, 0, 0)',
      'Text action has a fill without hover or focus',
    );
    check(
      getComputedStyle(textAction.querySelector('.tmt-ui-action-label')!).backgroundColor ===
        'rgba(0, 0, 0, 0)',
      'Text action label has a fill',
    );
    if (textAction.disabled)
      check(
        getComputedStyle(textAction).color === tokenColor('--tmt-ui-color-disabled-text'),
        'Disabled text action lost its muted text',
      );
  }
  await act(() =>
    root.render(
      <>
        {field(false)}
        <div className="fixture-actions">
          <BrowserAction
            type="button"
            label="Reconnect"
            variant="primary"
            busyMark="◌"
            onActivate={onActivate}
          />
          <BrowserAction
            type="button"
            label="Change recipient"
            variant="text"
            onActivate={onActivate}
          />
          {action({ pressed: true })}
          <BrowserIconAction
            type="button"
            label="Delete message"
            variant="destructive"
            icon="×"
            onActivate={onActivate}
          />
          <BrowserIconAction
            type="button"
            label="Post message"
            variant="primary"
            icon="+"
            disabled
            disabledReason="Access is read only"
            disabledReasonId="live-disabled-reason"
            onActivate={onActivate}
          />
        </div>
      </>,
    ),
  );
  result.textContent = `Passed ${assertions} lifecycle assertions`;
  result.dataset.parentEscapes = String(parentEscapes);
}
document.querySelector<HTMLButtonElement>('#run')!.addEventListener('click', () => {
  result.textContent = 'Running';
  void run().catch((error: unknown) => {
    result.textContent = `Failed: ${error instanceof Error ? error.message : String(error)}`;
  });
});
