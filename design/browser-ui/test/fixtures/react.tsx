import { act, createRef } from 'react';
import type { MouseEvent } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserAction, BrowserField, BrowserIconAction } from '../../src/react.js';

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
    !tooltip.matches(':popover-open') && parentEscapes === 0,
    'Escape leaked past the open tooltip',
  );
  await act(() =>
    button.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
  );
  check(parentEscapes === 1, 'Closed tooltip swallowed parent Escape');
  await act(() => button.dispatchEvent(new PointerEvent('pointerout', { bubbles: true })));
  await act(() => button.dispatchEvent(new PointerEvent('pointerover', { bubbles: true })));
  check(tooltip.matches(':popover-open'), 'Pointer return did not reopen the tooltip');
  await act(() => root.render(null));
  check(!tooltip.isConnected, 'Tooltip survived component removal');
  await act(() =>
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })),
  );
  check(parentEscapes === 2, 'Tooltip listener survived component removal');
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
