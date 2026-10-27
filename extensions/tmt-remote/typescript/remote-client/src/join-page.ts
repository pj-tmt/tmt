/** In-app invitation guidance; native join remains the only ceremony owner. */
import { browserUiClasses as c } from '@tmt/browser-ui/static';
import { createIcon } from '@tmt/browser-ui/glyph';
import { copyInvitation, invitationFromInput, MAX_INVITATION_INPUT } from './invite-page.js';
import { commandCopy } from './command-copy.js';

function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = ''): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  element.textContent = text;
  return element;
}

/** One route-scoped private interaction. Navigation never certifies enrollment. */
export function joinPage(
  target: HTMLElement,
  initial: string,
  navigateHome: () => void,
  initialInvalid = false,
): () => void {
  const lifetime = new AbortController();
  const { signal } = lifetime;
  let token = '';
  let expires = 0;
  let step = 0;
  let furthest = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0;
  const names = ['Invitation', 'Your terminal', 'Sending'];
  const chips = node('ol');
  chips.className = 'create-steps';
  chips.setAttribute('aria-label', 'Join guide progress');
  chips.setAttribute(
    'aria-description',
    'Done marks visited guide steps, not completed enrollment.',
  );
  const panels = names.map((name) => {
    const panel = node('section');
    panel.className = 'create-step-panel';
    panel.hidden = true;
    const heading = node('h2', name);
    heading.className = 'create-step-heading';
    panel.append(heading);
    return panel;
  });
  const buttons: HTMLButtonElement[] = [];
  const action = (label: string, run: () => void, primary = false) => {
    const button = node('button');
    button.type = 'button';
    button.className = c.action;
    if (primary) button.dataset.variant = 'primary';
    const text = node('span', label);
    text.className = c.actionLabel;
    button.append(text);
    button.addEventListener('click', run, { signal });
    return button;
  };
  for (const [index, name] of names.entries()) {
    const item = node('li');
    const button = action(`${index + 1} ${name}`, () => show(index));
    button.className = `${c.action} ${c.chip} create-step`;
    item.append(button);
    chips.append(item);
    buttons.push(button);
  }
  const card = node('div');
  card.className = c.card;
  const label = node('label');
  label.className = c.field;
  const fieldName = node('span', 'Invitation link or code');
  fieldName.className = c.fieldLabel;
  const input = node('input');
  input.type = 'password';
  input.maxLength = MAX_INVITATION_INPUT;
  input.autocomplete = 'off';
  input.spellcheck = false;
  input.className = c.fieldControl;
  input.value = initial;
  initial = '';
  const hint = node('span', 'Paste the private link or code from your inviter.');
  hint.className = c.fieldDescription;
  label.append(fieldName, input, hint);
  const error = node('p');
  error.className = c.fieldError;
  error.setAttribute('role', 'status');
  card.append(
    label,
    error,
    node(
      'p',
      'Each invitation can be used once and expires. Ask your inviter for a fresh one if needed.',
    ),
  );
  card.lastElementChild!.classList.add('join-note');
  panels[0]!.append(card);

  const terminalCard = node('div');
  terminalCard.className = c.card;
  terminalCard.append(node('p', 'Copy this command and run it in this machine’s terminal.'));
  const command = node('div');
  command.className = c.command;
  const code = node('code');
  code.className = c.commandText;
  for (const word of ['tmt', 'remote', 'join', '[private code]']) {
    if (code.childNodes.length) code.append(' ');
    const span = node('span', word);
    span.className = 'create-command-token';
    code.append(span);
  }
  const copy = node('button');
  copy.type = 'button';
  copy.className = c.commandCopy;
  copy.setAttribute('aria-label', 'Copy join command');
  copy.append(createIcon('copy'), 'Copy');
  const feedback = node('span');
  feedback.className = c.commandFeedback;
  feedback.setAttribute('role', 'status');
  command.append(code, copy, feedback);
  const expiryLine = node('p');
  const expired = node('p');
  expired.setAttribute('role', 'status');
  terminalCard.append(
    command,
    expiryLine,
    expired,
    node(
      'p',
      'Your terminal shows four words. Compare them with your inviter over a trusted conversation; your inviter confirms in their own terminal.',
    ),
  );
  panels[1]!.append(terminalCard);
  const sendingCard = node('div');
  sendingCard.className = c.card;
  sendingCard.append(
    node(
      'p',
      'Sending starts off. After confirming your invitation, your inviter can enable it for your member entry.',
    ),
    commandCopy('Please enable sending for my member entry after confirming this invitation.'),
    node(
      'p',
      'If the terminal says the invitation expired or was already used, ask your inviter for a fresh one.',
    ),
  );
  sendingCard.lastElementChild!.classList.add('join-note');
  panels[2]!.append(sendingCard);

  const actions = node('div');
  actions.className = `${c.actions} create-actions`;
  const cancel = action('Cancel', () => {
    clear();
    navigateHome();
  });
  const back = action('Back', () => show(step - 1));
  const next = action(
    'Continue',
    () => {
      if (step === 0) {
        try {
          const selected = invitationFromInput(input.value);
          token = selected.token;
          expires = selected.expiry;
          error.textContent = '';
          tick();
        } catch {
          clear();
          error.textContent =
            'This invitation is missing, invalid or expired. Use the original code in your terminal, or ask your inviter for a new one.';
          return;
        }
      }
      if (step === 2) {
        clear();
        navigateHome();
        return;
      }
      show(step + 1);
    },
    true,
  );
  actions.append(cancel, back, next);
  target.replaceChildren(chips, ...panels, actions);

  function clear(clearInput = true): void {
    generation++;
    token = '';
    expires = 0;
    if (clearInput) input.value = '';
    feedback.textContent = '';
    copy.disabled = true;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
  }
  function tick(): void {
    if (timer !== undefined) clearTimeout(timer);
    if (!token || signal.aborted) return;
    const remaining = Math.max(0, expires - Date.now());
    if (remaining === 0) {
      clear();
      expiryLine.textContent = '';
      expired.textContent =
        'This invitation has reached its expiry. Ask your inviter for a new one; this page cannot confirm cloud cleanup.';
      return;
    }
    expired.textContent = '';
    copy.disabled = false;
    const seconds = Math.ceil(remaining / 1000);
    expiryLine.textContent = `Expires in ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')} · based on the link; not checked with the Remote.`;
    timer = setTimeout(tick, Math.min(1000, remaining));
  }
  function armExpiry(value: string): boolean {
    try {
      const selected = invitationFromInput(value);
      token = selected.token;
      expires = selected.expiry;
      tick();
      return true;
    } catch {
      // Input may still be incomplete; explicit Continue reports the refusal.
      return false;
    }
  }
  function show(index: number): void {
    if (index < 0 || index > 2 || index > furthest + 1) return;
    step = index;
    furthest = Math.max(furthest, index);
    for (const [i, panel] of panels.entries()) {
      panel.hidden = i !== index;
      const button = buttons[i]!;
      button.disabled = i > furthest;
      button.dataset.stepState = i === index ? 'current' : i < furthest ? 'done' : 'upcoming';
      if (i === index) button.setAttribute('aria-current', 'step');
      else button.removeAttribute('aria-current');
      button.querySelector(`.${c.actionLabel}`)!.textContent =
        `${i < furthest ? '✓' : i + 1} ${names[i]}`;
    }
    back.hidden = index === 0;
    next.querySelector(`.${c.actionLabel}`)!.textContent = index === 2 ? 'Done' : 'Continue';
  }
  input.addEventListener(
    'input',
    () => {
      clear(false);
      error.textContent = '';
      armExpiry(input.value);
    },
    { signal },
  );
  copy.addEventListener(
    'click',
    () => {
      if (!token || Date.now() >= expires) {
        tick();
        return;
      }
      const active = generation;
      void copyInvitation(token)
        .then(() => {
          if (!signal.aborted && generation === active)
            feedback.textContent = 'Command copied. Run it in your terminal.';
        })
        .catch(() => {
          if (!signal.aborted && generation === active)
            feedback.textContent =
              'Copy failed. Use the original code in your terminal, or ask your inviter for a new one.';
        });
    },
    { signal },
  );
  window.addEventListener(
    'pagehide',
    () => {
      clear();
      lifetime.abort();
    },
    { signal, once: true },
  );
  if (initialInvalid || (input.value && !armExpiry(input.value))) {
    clear();
    error.textContent =
      'This invitation is missing, invalid or expired. Use the original code in your terminal, or ask your inviter for a new one.';
  }
  show(0);
  return () => {
    clear();
    lifetime.abort();
    target.replaceChildren();
  };
}
