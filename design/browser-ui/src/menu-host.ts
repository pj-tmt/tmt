import { createMenuController, type MenuEntry } from './menu-controller.js';
import { createIcon } from './glyph.js';
import { bindIconActionTooltip } from './icon-action-tooltip.js';
import { browserUiClasses as c } from './static.js';

export interface BrowserMenuHostItem<Key extends string = string> extends MenuEntry<Key> {
  /** Passive presentation, never a second interactive control. */
  content?: Node;
  disabledReason?: string;
  checked?: boolean;
  role?: 'menuitem' | 'menuitemradio' | 'menuitemcheckbox';
  separated?: boolean;
}
export interface BrowserMenuHostOptions<Key extends string = string> {
  label: string;
  items: readonly BrowserMenuHostItem<Key>[];
  onSelect(key: Key, event: MouseEvent): void;
  triggerText?: string;
  icon?: Node;
  description?: string | Node;
  disabled?: boolean;
  onOpenChange?(open: boolean): void;
}
export interface BrowserMenuHost<Key extends string = string> {
  update(options: BrowserMenuHostOptions<Key>): void;
  open(): void;
  close(): void;
  dispose(): void;
}

/** Mount the existing menu presentation without a framework; hosts retain values and commands. */
export function mountBrowserMenu<Key extends string>(
  container: HTMLElement,
  initial: BrowserMenuHostOptions<Key>,
): BrowserMenuHost<Key> {
  const document = container.ownerDocument;
  const id = `tmt-menu-${document.defaultView!.crypto.randomUUID()}`;
  const root = document.createElement('span');
  root.className = 'tmt-ui-menu';
  const wrapper = document.createElement('span');
  const trigger = document.createElement('button');
  trigger.type = 'button';
  trigger.setAttribute('aria-haspopup', 'menu');
  const tooltip = document.createElement('span');
  tooltip.className = c.iconActionTooltip;
  tooltip.setAttribute('popover', 'manual');
  tooltip.setAttribute('aria-hidden', 'true');
  const tooltipLabel = document.createElement('span');
  tooltipLabel.className = c.iconActionTooltipLabel;
  tooltip.append(tooltipLabel);
  wrapper.append(trigger, tooltip);
  const surface = document.createElement('dialog');
  surface.className = 'tmt-ui-menu-surface';
  surface.tabIndex = -1;
  surface.hidden = true;
  const menu = document.createElement('div');
  menu.id = id;
  menu.setAttribute('role', 'menu');
  menu.tabIndex = -1;
  const description = document.createElement('div');
  description.id = `${id}-description`;
  description.className = 'tmt-ui-menu-description';
  surface.append(menu);
  root.append(wrapper, surface);
  let options = initial;
  let disposed = false;
  let help: ReturnType<typeof bindIconActionTooltip> | undefined;
  const rows = new Map<
    Key,
    {
      root: HTMLDivElement;
      button: HTMLButtonElement;
      label: HTMLSpanElement;
      check: HTMLSpanElement;
      reason: HTMLSpanElement;
    }
  >();
  const controller = createMenuController<Key, MouseEvent>({
    items: () => options.items,
    disabled: () => options.disabled === true,
    hasDescription: () => Boolean(options.description),
    trigger: () => trigger,
    surface: () => surface,
    entries: () => options.items.map((item) => rows.get(item.key)?.button ?? null),
    selection: () => options.onSelect,
    onOpenChange: (open) => options.onOpenChange?.(open),
  });
  const display = (target: HTMLElement, content: string | Node) =>
    target.replaceChildren(
      typeof content === 'string' ? document.createTextNode(content) : content,
    );
  const validate = (next: BrowserMenuHostOptions<Key>) => {
    if (!next.label.trim()) throw new Error('Menu requires a nonempty label');
    if (new Set(next.items.map((item) => item.key)).size !== next.items.length)
      throw new Error('Menu item keys must be unique');
  };
  function renderOptions() {
    const iconOnly = options.triggerText === undefined;
    wrapper.className = iconOnly ? c.iconAction : '';
    trigger.className = iconOnly ? `${c.action} ${c.iconActionControl}` : 'tmt-ui-menu-trigger';
    if (iconOnly) trigger.dataset.variant = 'text';
    else delete trigger.dataset.variant;
    trigger.setAttribute('aria-label', options.label);
    tooltipLabel.textContent = options.label;
    tooltip.hidden = !iconOnly;
    surface.setAttribute('aria-label', options.label);
    menu.setAttribute('aria-label', options.label);
    trigger.disabled = !controller.canOpen();
    if (iconOnly) {
      const icon = document.createElement('span');
      icon.className = c.iconActionIcon;
      icon.setAttribute('aria-hidden', 'true');
      icon.append(options.icon ?? createIcon('ellipsis', document));
      trigger.replaceChildren(icon);
      help ??= bindIconActionTooltip(wrapper, trigger, tooltip);
    } else {
      help?.dispose();
      help = undefined;
      const label = document.createElement('span');
      label.className = 'tmt-ui-menu-value';
      label.textContent = options.triggerText!;
      trigger.replaceChildren(label, createIcon('chevron-down', document));
    }
    if (options.description) {
      display(description, options.description);
      menu.setAttribute('aria-describedby', description.id);
    } else menu.removeAttribute('aria-describedby');
    const removed = new Set(rows.keys());
    const children: Node[] = options.description ? [description] : [];
    for (const [index, item] of options.items.entries()) {
      let row = rows.get(item.key);
      if (!row) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'tmt-ui-menu-item';
        button.tabIndex = -1;
        button.onclick = (event) => controller.select(item.key, event);
        button.onfocus = () => controller.focused(item.key);
        row = {
          root: document.createElement('div'),
          button,
          label: document.createElement('span'),
          check: document.createElement('span'),
          reason: document.createElement('span'),
        };
        row.check.setAttribute('aria-hidden', 'true');
        row.check.textContent = '✓';
        row.reason.className = 'tmt-ui-menu-reason';
        row.root.append(button);
        rows.set(item.key, row);
      }
      removed.delete(item.key);
      display(row.label, item.content ?? item.label);
      row.button.replaceChildren(row.label, ...(item.checked ? [row.check] : []));
      row.button.disabled = item.disabled === true;
      row.button.setAttribute(
        'role',
        item.role ?? (item.checked === undefined ? 'menuitem' : 'menuitemradio'),
      );
      if (item.checked === undefined) row.button.removeAttribute('aria-checked');
      else row.button.setAttribute('aria-checked', String(item.checked));
      if (item.separated) row.button.dataset.separated = 'true';
      else delete row.button.dataset.separated;
      if (item.disabledReason) {
        row.reason.id = `${id}-reason-${index}`;
        row.reason.textContent = item.disabledReason;
        row.button.setAttribute('aria-describedby', row.reason.id);
        if (!row.reason.isConnected) row.root.append(row.reason);
      } else {
        row.reason.remove();
        row.button.removeAttribute('aria-describedby');
      }
      children.push(row.root);
    }
    for (const key of removed) {
      rows.get(key)!.root.remove();
      rows.delete(key);
    }
    // Keep surviving keyed rows connected so an observation refresh retains their focus.
    for (const [index, child] of children.entries())
      if (menu.childNodes[index] !== child)
        menu.insertBefore(child, menu.childNodes[index] ?? null);
    while (menu.childNodes.length > children.length) menu.lastChild!.remove();
    controller.refresh();
    help?.refresh();
  }
  function renderState() {
    const { open, present } = controller.getSnapshot();
    trigger.setAttribute('aria-expanded', String(open));
    if (open) trigger.setAttribute('aria-controls', id);
    else trigger.removeAttribute('aria-controls');
    surface.hidden = !present;
    surface.dataset.state = open ? 'open' : 'closed';
    help?.refresh();
    if (open) controller.attachSurface();
  }
  trigger.onclick = (event) => {
    if (event.isTrusted) {
      help?.dismiss();
      controller.open();
    }
  };
  trigger.onkeydown = controller.triggerKey;
  surface.onkeydown = controller.menuKey;
  surface.onclose = controller.nativeClose;
  surface.oncancel = controller.cancel;
  surface.onpointerdown = controller.pointerDown;
  surface.onclick = controller.surfaceClick;
  validate(initial);
  container.append(root);
  renderOptions();
  const unsubscribe = controller.subscribe(renderState);
  renderState();
  return {
    update(next) {
      if (!disposed) {
        validate(next);
        options = next;
        renderOptions();
        renderState();
      }
    },
    open: () => controller.open(),
    close: () => controller.close(),
    dispose() {
      if (disposed) return;
      disposed = true;
      controller.dispose();
      unsubscribe();
      help?.dispose();
      root.remove();
      rows.clear();
    },
  };
}
