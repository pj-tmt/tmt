import { placeIconActionTooltip } from '../../src/icon-action-tooltip.ts';
import { checkFieldFocus } from './field-focus.ts';

document.querySelector('#run-focus').addEventListener('click', () => {
  const result = document.querySelector('#focus-result');
  try {
    const count = checkFieldFocus([
      document.querySelector('#native'),
      document.querySelector('#editable'),
    ]);
    result.textContent = `Passed ${count} field focus assertions`;
  } catch (error) {
    result.textContent = `Failed: ${error.message}`;
  }
});

for (const variant of ['text', 'primary', 'destructive']) {
  for (const suppliedMark of [false, true]) {
    for (const busy of [false, true]) {
      const cell = document.createElement('div');
      cell.className = 'fixture-action';
      const caption = document.createElement('span');
      caption.textContent = `${variant}, ${suppliedMark ? 'mark' : 'no mark'}, ${busy ? 'busy' : 'ready'}`;
      const button = document.createElement('button');
      button.className = 'tmt-ui-action';
      button.type = 'button';
      button.dataset.variant = variant;
      button.disabled = busy;
      if (busy) button.setAttribute('aria-busy', 'true');
      if (suppliedMark) {
        const mark = document.createElement('span');
        mark.className = 'tmt-ui-action-mark';
        mark.setAttribute('aria-hidden', 'true');
        mark.dataset.busy = String(busy);
        mark.textContent = '◌';
        button.append(mark);
      }
      const label = document.createElement('span');
      label.className = 'tmt-ui-action-label';
      label.textContent = 'Reconnect';
      button.append(label);
      cell.append(caption, button);
      document.querySelector('#busy-actions').append(cell);
    }
  }
}

// Fixture host for the static markup contract; no React or product runtime.
function action(label, variant, state, disabled = false) {
  const cell = document.createElement('div');
  cell.className = 'fixture-action';
  const caption = document.createElement('span');
  caption.textContent = state;
  const wrapper = document.createElement('span');
  wrapper.className = 'tmt-ui-icon-action';
  const button = document.createElement('button');
  button.className = 'tmt-ui-action tmt-ui-icon-action-control';
  button.type = 'button';
  button.dataset.variant = variant;
  button.dataset.fixtureState = state;
  button.disabled = disabled;
  button.setAttribute('aria-label', label);
  if (state === 'pressed' || disabled) button.setAttribute('aria-pressed', 'true');
  const icon = document.createElement('span');
  icon.className = 'tmt-ui-icon-action-icon';
  icon.setAttribute('aria-hidden', 'true');
  icon.textContent = state === 'destructive' ? '×' : '+';
  button.append(icon);
  const tooltip = document.createElement('span');
  tooltip.className = 'tmt-ui-icon-action-tooltip';
  tooltip.popover = 'manual';
  tooltip.setAttribute('aria-hidden', 'true');
  const copy = document.createElement('span');
  copy.className = 'tmt-ui-icon-action-tooltip-label';
  copy.textContent = label;
  tooltip.append(copy);
  wrapper.append(button, tooltip);
  cell.append(caption, wrapper);
  if (disabled) {
    const reason = document.createElement('span');
    reason.className = 'tmt-ui-field-description';
    reason.id = 'static-disabled-reason';
    reason.textContent = 'Access is read only';
    button.setAttribute('aria-describedby', reason.id);
    cell.append(reason);
  }
  let dispose;
  let hovered = false;
  let focused = false;
  let dismissed = false;
  const escape = (event) => {
    if (event.key !== 'Escape' || !tooltip.matches(':popover-open')) return;
    event.stopPropagation();
    dismissed = true;
    sync();
  };
  function sync() {
    const open = (hovered || focused) && !dismissed;
    if (open === tooltip.matches(':popover-open')) return;
    if (open) {
      tooltip.showPopover();
      dispose = placeIconActionTooltip(button, tooltip);
      document.addEventListener('keydown', escape, true);
    } else {
      dispose?.();
      document.removeEventListener('keydown', escape, true);
      tooltip.hidePopover();
    }
  }
  wrapper.addEventListener('pointerenter', () => {
    hovered = true;
    dismissed = false;
    sync();
  });
  wrapper.addEventListener('pointerleave', () => {
    hovered = false;
    sync();
  });
  button.addEventListener('focus', () => {
    focused = button.matches(':focus-visible');
    dismissed = false;
    sync();
  });
  button.addEventListener('blur', () => {
    focused = false;
    sync();
  });
  return cell;
}
for (const [label, variant, state, disabled] of [
  ['Show details', 'text', 'default', false],
  ['Show details', 'text', 'hover', false],
  ['Show details', 'text', 'focus', false],
  ['Show details', 'text', 'pressed', false],
  ['Post message', 'primary', 'primary', false],
  ['Delete message', 'destructive', 'destructive', false],
  ['Show details', 'text', 'disabled', true],
])
  document.querySelector('#states').append(action(label, variant, state, disabled));
for (const label of [
  'A long label at the left viewport edge',
  'A long label at the right viewport edge',
]) {
  document.querySelector('#edges').append(action(label, 'text', 'edge'));
}
