import type { ShellMenuAction, ShellMenuGroup, ShellMenuItem } from './shell.js';
import type { ShellIcon } from './shell-icons.js';

export interface ShellMenuInteractions {
  decorate(row: HTMLLIElement, anchor: HTMLAnchorElement, item: ShellMenuItem): void;
  dispose(): void;
}

/** One group's transient controls. The host's latest model is the only order and command owner. */
export function createShellMenuInteractions(
  current: () => ShellMenuGroup | undefined,
  drawIcon: (name: ShellIcon) => SVGSVGElement,
  activate: (action: ShellMenuAction) => void,
  announce: (message: string) => void,
): ShellMenuInteractions {
  const lifetime = new AbortController();
  const listen = { signal: lifetime.signal };
  let source: HTMLLIElement | undefined;
  let target: HTMLLIElement | undefined;
  let fromId: string | undefined;
  const cancel = () => {
    if (source) delete source.dataset.dragging;
    if (target) delete target.dataset.dropEdge;
    source = target = undefined;
    fromId = undefined;
  };
  const requestMove = (from: string, to: string) => {
    const group = current();
    if (!group?.reorder || from === to) return;
    const fromIndex = group.items.findIndex((item) => item.id === from);
    const toIndex = group.items.findIndex((item) => item.id === to);
    if (fromIndex < 0 || toIndex < 0) return;
    const label = group.items[fromIndex]!.label;
    group.reorder(from, to);
    // A callback is a request, not proof that the host persisted or even accepted the move.
    announce(`Move requested: ${label}, position ${toIndex + 1} of ${group.items.length}.`);
  };
  return {
    decorate(row, anchor, item) {
      const doc = row.ownerDocument;
      row.classList.add('tmt-ui-rail-menu-row');
      row.dataset.menuItem = item.id;
      anchor.dataset.railPart = 'link';
      const button = (label: string, name: ShellIcon, part: string) => {
        const control = doc.createElement('button');
        control.type = 'button';
        control.className = 'tmt-ui-rail-group-action tmt-ui-rail-row-action';
        control.title = label;
        control.setAttribute('aria-label', label);
        control.dataset.railPart = part;
        control.append(drawIcon(name));
        return control;
      };
      const actions = item.actions?.slice(0, 2) ?? [];
      if (actions.length) {
        row.dataset.actions = String(actions.length);
        const commands = doc.createElement('div');
        commands.className = 'tmt-ui-rail-row-actions';
        for (const [index, action] of actions.entries()) {
          const control = button(action.label, action.icon, `action-${index}`);
          control.addEventListener(
            'click',
            (event) => {
              event.stopPropagation();
              const latest = current()?.items.find((entry) => entry.id === item.id)?.actions?.[
                index
              ];
              if (latest) activate(latest);
            },
            listen,
          );
          commands.append(control);
        }
        row.append(commands);
      }
      if (current()?.reorder) {
        row.dataset.reorderable = '';
        anchor.draggable = false;
        anchor.setAttribute('aria-keyshortcuts', 'Alt+ArrowUp Alt+ArrowDown');
        const handle = button(`Reorder ${item.label}`, 'grip-vertical', 'drag');
        handle.classList.add('tmt-ui-rail-drag');
        handle.draggable = true;
        handle.setAttribute('aria-keyshortcuts', 'Alt+ArrowUp Alt+ArrowDown');
        handle.addEventListener(
          'click',
          (event) => {
            event.preventDefault();
            event.stopPropagation();
          },
          listen,
        );
        handle.addEventListener(
          'dragstart',
          (event) => {
            const group = current();
            if (!group?.reorder || !group.items.some((entry) => entry.id === item.id)) {
              event.preventDefault();
              return;
            }
            cancel();
            source = row;
            fromId = item.id;
            row.dataset.dragging = '';
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = 'move';
              event.dataTransfer.setData('application/x-tmt-shell-row', 'move');
            }
          },
          listen,
        );
        handle.addEventListener('dragend', cancel, listen);
        const slot = doc.createElement('span');
        slot.className = 'tmt-ui-swap';
        const leading = anchor.querySelector<SVGSVGElement>(':scope > svg.tmt-ui-shell-icon');
        const rest = leading ?? doc.createElement('span');
        if (leading) {
          // The interactive grip is a sibling of the link, never a button nested inside it.
          const placeholder = doc.createElement('span');
          placeholder.className = 'tmt-ui-shell-icon';
          placeholder.setAttribute('aria-hidden', 'true');
          anchor.replaceChild(placeholder, leading);
        }
        rest.setAttribute('data-swap', 'rest');
        rest.setAttribute('aria-hidden', 'true');
        handle.setAttribute('data-swap', 'reveal');
        slot.append(rest, handle);
        row.append(slot);
      }
      row.addEventListener(
        'keydown',
        (event) => {
          if (event.key === 'Escape' && fromId !== undefined) {
            cancel();
            event.stopPropagation();
            return;
          }
          if (
            !event.altKey ||
            event.ctrlKey ||
            event.metaKey ||
            event.shiftKey ||
            !['ArrowUp', 'ArrowDown'].includes(event.key)
          )
            return;
          const group = current();
          if (!group?.reorder) return;
          const index = group.items.findIndex((entry) => entry.id === item.id);
          if (index < 0) return;
          event.preventDefault();
          event.stopPropagation();
          const next = group.items[index + (event.key === 'ArrowUp' ? -1 : 1)];
          if (next) requestMove(item.id, next.id);
          else announce(`${item.label} is already ${event.key === 'ArrowUp' ? 'first' : 'last'}.`);
        },
        listen,
      );
      row.addEventListener(
        'dragover',
        (event) => {
          const group = current();
          const from = group?.items.findIndex((entry) => entry.id === fromId) ?? -1;
          const to = group?.items.findIndex((entry) => entry.id === item.id) ?? -1;
          // Only a handle started by this same live group can supply a move, never external data.
          if (!group?.reorder || from < 0 || to < 0 || from === to) return;
          event.preventDefault();
          if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
          if (target) delete target.dataset.dropEdge;
          target = row;
          row.dataset.dropEdge = from < to ? 'after' : 'before';
        },
        listen,
      );
      row.addEventListener(
        'dragleave',
        (event) => {
          if (!row.contains(event.relatedTarget as Node | null)) delete row.dataset.dropEdge;
        },
        listen,
      );
      row.addEventListener(
        'drop',
        (event) => {
          if (fromId === undefined) return;
          event.preventDefault();
          event.stopPropagation();
          const from = fromId;
          cancel();
          requestMove(from, item.id);
        },
        listen,
      );
    },
    dispose() {
      lifetime.abort();
      cancel();
    },
  };
}
