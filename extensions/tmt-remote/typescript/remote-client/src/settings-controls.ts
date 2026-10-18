/** Static settings choices; values are drafts, never management authority. */
export function settingsChoice(trigger: HTMLButtonElement, surface: HTMLElement): () => void {
  const options = [...surface.querySelectorAll<HTMLButtonElement>('[role=option]')];
  const enabled = () => options.filter((option) => !option.disabled);
  const opened = () => surface.matches(':popover-open');
  function close(refocus = false): void {
    if (opened()) surface.hidePopover();
    trigger.setAttribute('aria-expanded', 'false');
    if (refocus && !trigger.disabled) trigger.focus({ preventScroll: true });
  }
  function place(): void {
    const anchor = trigger.getBoundingClientRect();
    const viewport = window.visualViewport;
    const left = viewport?.offsetLeft ?? 0;
    const top = viewport?.offsetTop ?? 0;
    const width = viewport?.width ?? document.documentElement.clientWidth;
    const height = viewport?.height ?? innerHeight;
    surface.style.maxWidth = `${Math.max(0, width - 16)}px`;
    surface.style.width = `${Math.min(anchor.width, width - 16)}px`;
    surface.style.maxHeight = `${height - 16}px`;
    const box = surface.getBoundingClientRect();
    const below = top + height - anchor.bottom - 8;
    const above = anchor.top - top - 8;
    const up = box.height > below && above > below;
    surface.style.left = `${Math.max(left + 8, Math.min(anchor.left, left + width - box.width - 8))}px`;
    surface.style.maxHeight = `${Math.max(0, up ? above : below)}px`;
    surface.style.top = `${up ? Math.max(top + 8, anchor.top - Math.min(box.height, above)) : anchor.bottom}px`;
  }
  function open(last = false): void {
    if (trigger.disabled) return;
    if (!opened()) surface.showPopover();
    trigger.setAttribute('aria-expanded', 'true');
    place();
    search = '';
    const items = enabled();
    const selected = items.find((option) => option.dataset.value === trigger.value);
    (selected ?? (last ? items.at(-1) : items[0]))?.focus();
  }
  let pointerOpen = false;
  let search = '';
  let searchAt = 0;
  trigger.addEventListener('pointerdown', () => {
    pointerOpen = opened();
  });
  trigger.addEventListener('click', (event) => {
    const dismiss = event.detail > 0 && pointerOpen;
    pointerOpen = false;
    if (dismiss || opened()) close();
    else open();
  });
  trigger.addEventListener('keydown', (event) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      open(event.key === 'ArrowUp');
    }
  });
  for (const option of options) {
    option.addEventListener('click', () => {
      if (trigger.disabled || option.disabled) return;
      trigger.value = option.dataset.value!;
      close(true);
      trigger.dispatchEvent(new Event('change', { bubbles: true }));
    });
  }
  surface.addEventListener('toggle', () => {
    trigger.setAttribute('aria-expanded', String(opened()));
  });
  surface.addEventListener('focusin', () => {
    for (const option of options) option.tabIndex = option === document.activeElement ? 0 : -1;
  });
  surface.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' || event.key === 'Tab') {
      if (event.key === 'Escape') event.preventDefault();
      // Continue Tab from the trigger's position, not from a now-hidden option.
      close(true);
      return;
    }
    if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    const items = enabled();
    const at = items.findIndex((option) => option === document.activeElement);
    let next: HTMLButtonElement | undefined;
    if (event.key === 'ArrowDown') next = items[(at + 1) % items.length];
    else if (event.key === 'ArrowUp') next = items[(at - 1 + items.length) % items.length];
    else if (event.key === 'Home') next = items[0];
    else if (event.key === 'End') next = items.at(-1);
    else if (event.key.length === 1 && event.key !== ' ') {
      const now = performance.now();
      search = (now - searchAt > 700 ? '' : search) + event.key.toLowerCase();
      searchAt = now;
      next = [...items.slice(at + 1), ...items.slice(0, at + 1)].find((option) =>
        option.textContent?.trim().toLowerCase().startsWith(search),
      );
    } else return;
    event.preventDefault();
    next?.focus();
  });
  const reposition = () => {
    if (opened()) place();
  };
  window.addEventListener('resize', reposition);
  window.addEventListener('scroll', reposition, true);
  window.visualViewport?.addEventListener('resize', reposition);
  window.visualViewport?.addEventListener('scroll', reposition);
  return () => {
    for (const option of options) {
      option.disabled = trigger.disabled || option.dataset.value === 'default';
      option.setAttribute('aria-selected', String(option.dataset.value === trigger.value));
    }
    trigger.querySelector('[data-choice-value]')!.textContent =
      options.find((option) => option.dataset.value === trigger.value)?.textContent?.trim() ?? '';
    if (trigger.disabled) close();
  };
}
