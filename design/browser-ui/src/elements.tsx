import { createElement, useId, useLayoutEffect, useRef } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { Check, Plus, Pencil, Trash2, Ellipsis, Settings, Info, X, Link } from 'lucide-react';
import { BrowserAction } from './action.js';
import { BrowserIconAction } from './icon-action.js';
import { BrowserField } from './field.js';
import { BrowserToggle } from './toggle.js';
import { BrowserCheckbox } from './checkbox.js';
import { BrowserMenu } from './menu.js';
import { BrowserTabs } from './tabs.js';
import { BrowserDialog } from './dialog.js';
import { BrowserChips, BrowserChip } from './chips.js';
import { BrowserNotice } from './notice.js';
import { BusyMark } from './spinner.js';
import type { BrowserActionVariant, BrowserNoticeTone } from './static.js';
import {
  browserElements,
  elementAttributeIssue,
  type BrowserElementDefinition,
} from './elements-manifest.js';

export { browserElements } from './elements-manifest.js';
const icons = {
  check: Check,
  plus: Plus,
  pencil: Pencil,
  'trash-2': Trash2,
  ellipsis: Ellipsis,
  settings: Settings,
  info: Info,
  x: X,
  link: Link,
};
type StateValue = string | boolean | readonly string[];
interface ElementOwner {
  element: HTMLElement;
  definition: BrowserElementDefinition;
  source: readonly Node[];
  slots: ReadonlyMap<string, readonly Node[]>;
  text: string;
  items: readonly { id: string; label: string; node: Element; nodes: readonly Node[] }[];
  state: Map<string, StateValue>;
  render(): void;
}
function attribute(owner: ElementOwner, name: string, fallback = ''): string {
  return owner.element.getAttribute(name) ?? fallback;
}
function stringState(owner: ElementOwner, key: string, fallback: string): string {
  const value = owner.state.get(key);
  return typeof value === 'string' ? value : fallback;
}
function boolState(owner: ElementOwner, key: string): boolean {
  const value = owner.state.get(key);
  return typeof value === 'boolean' ? value : owner.element.hasAttribute(key);
}
function changed(owner: ElementOwner, key: string, value: StateValue): void {
  owner.state.set(key, value);
  owner.render();
}
const NO_NODES: readonly Node[] = [];
function authoredSlot(owner: ElementOwner, name: string): readonly Node[] {
  return owner.slots.get(name) ?? NO_NODES;
}
function items(
  owner: ElementOwner,
): readonly { id: string; label: string; node: Element; nodes: readonly Node[] }[] {
  return owner.items;
}
/** Move the authored nodes once, rather than turning slots into copied HTML or duplicate IDs. */
function AuthoredSlot({ nodes }: { nodes: readonly Node[] }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    ref.current!.replaceChildren(...nodes);
  }, [nodes]);
  return <div ref={ref} style={{ display: 'contents' }} />;
}
function valid(owner: ElementOwner): boolean {
  for (const name of owner.element.getAttributeNames()) {
    if (elementAttributeIssue(owner.definition, name, attribute(owner, name))) return false;
  }
  for (const [name, rule] of Object.entries(owner.definition.attributes)) {
    if (rule.required && !attribute(owner, name).trim()) return false;
  }
  if (owner.definition.textLabel && !(attribute(owner, 'label') || owner.text).trim()) return false;
  for (const node of owner.source) {
    if (
      node instanceof Element &&
      node.hasAttribute('slot') &&
      !owner.definition.slots.includes(node.getAttribute('slot')!)
    )
      return false;
    if (
      node instanceof Element &&
      !node.hasAttribute('slot') &&
      !owner.definition.textLabel &&
      !owner.definition.slots.includes('body')
    )
      return false;
  }
  if (!owner.definition.textLabel && !owner.definition.slots.length && owner.text) return false;
  if (owner.definition.itemSlot) {
    const entries = items(owner);
    if (entries.length < (owner.definition.minimumItems ?? 0)) return false;
    if (owner.source.some((node) => node.nodeType === Node.TEXT_NODE && node.textContent?.trim()))
      return false;
    if (
      entries.some((item) => !item.id.trim() || /\s/.test(item.id) || !item.label) ||
      new Set(entries.map((item) => item.id)).size !== entries.length
    )
      return false;
    const selected = attribute(owner, 'value');
    if (selected && !entries.some((item) => item.id === selected)) return false;
  }
  return true;
}
function ElementView({ owner }: { owner: ElementOwner }) {
  const id = useId();
  if (!valid(owner)) return <span role="alert">Invalid {owner.definition.tag} markup.</span>;
  const label = (attribute(owner, 'label').trim() || owner.text).trim();
  const disabled = owner.element.hasAttribute('disabled');
  const body = () => <AuthoredSlot nodes={authoredSlot(owner, 'body')} />;
  const action = {
    type: 'button' as const,
    label,
    variant: attribute(owner, 'variant', 'secondary') as BrowserActionVariant,
    presentation: owner.element.hasAttribute('presentation') ? ('compact' as const) : undefined,
    disabled,
    busy: owner.element.hasAttribute('busy'),
    busyMark: owner.element.hasAttribute('busy') ? <BusyMark /> : undefined,
    disabledReason: owner.element.getAttribute('disabled-reason') ?? undefined,
    disabledReasonId: `${id}-reason`,
    onActivate: () => {
      owner.element.dispatchEvent(new CustomEvent('activate', { bubbles: true }));
    },
  };
  switch (owner.definition.tag) {
    case 'tmt-ui-button':
      return (
        <BrowserAction
          {...action}
          leadingIcon={
            owner.element.hasAttribute('icon')
              ? createElement(icons[attribute(owner, 'icon') as keyof typeof icons])
              : undefined
          }
        />
      );
    case 'tmt-ui-icon-button': {
      const icon = attribute(owner, 'icon', 'pencil') as keyof typeof icons;
      return (
        <BrowserIconAction
          {...action}
          variant={action.variant === 'ghost' ? 'secondary' : action.variant}
          icon={createElement(icons[icon])}
        />
      );
    }
    case 'tmt-ui-field':
      return (
        <BrowserField
          controlId={`${id}-input`}
          label={label}
          description={owner.element.getAttribute('description') ?? undefined}
          descriptionId={`${id}-description`}
          error={owner.element.getAttribute('error') ?? undefined}
          errorId={`${id}-error`}
          invalid={owner.element.hasAttribute('error')}
          renderControl={(props) => (
            <input
              {...props}
              disabled={disabled}
              placeholder={attribute(owner, 'placeholder')}
              value={stringState(owner, 'value', attribute(owner, 'value'))}
              onChange={(event) => changed(owner, 'value', event.target.value)}
            />
          )}
        />
      );
    case 'tmt-ui-toggle':
      return (
        <BrowserToggle
          label={label}
          pressed={boolState(owner, 'pressed')}
          disabled={disabled}
          onActivate={() => changed(owner, 'pressed', !boolState(owner, 'pressed'))}
        />
      );
    case 'tmt-ui-checkbox':
      return (
        <label>
          <BrowserCheckbox
            aria-label={label}
            checked={boolState(owner, 'checked')}
            disabled={disabled}
            indeterminate={boolState(owner, 'indeterminate')}
            onChange={(event) => {
              owner.state.set('indeterminate', false);
              changed(owner, 'checked', event.target.checked);
            }}
          />{' '}
          {label}
        </label>
      );
    case 'tmt-ui-menu': {
      const entries = items(owner);
      const selected = stringState(owner, 'value', attribute(owner, 'value', entries[0]!.id));
      return (
        <BrowserMenu
          label={label}
          disabled={disabled}
          valueLabel={entries.find((item) => item.id === selected)?.label}
          presentation={action.presentation}
          items={entries.map((item) => ({
            key: item.id,
            label: item.label,
            checked: item.id === selected,
            role: 'menuitemradio',
            disabled: item.node.hasAttribute('data-disabled'),
          }))}
          onSelect={(value) => changed(owner, 'value', value)}
        />
      );
    }
    case 'tmt-ui-tabs': {
      const entries = items(owner);
      const selected = stringState(owner, 'value', attribute(owner, 'value', entries[0]!.id));
      const tabs = entries.map((item) => ({
        id: item.id,
        label: item.label,
        tabId: `${id}-tab-${item.id}`,
        panelId: `${id}-panel-${item.id}`,
      }));
      return (
        <BrowserTabs
          label={label}
          tabs={tabs}
          selected={selected}
          variant={attribute(owner, 'variant', 'boxed') as 'boxed' | 'flush' | 'flat'}
          wrap={owner.element.hasAttribute('wrap')}
          onSelect={(value) => changed(owner, 'value', value)}
        >
          {(tab) => <AuthoredSlot nodes={entries.find((item) => item.id === tab.id)!.nodes} />}
        </BrowserTabs>
      );
    }
    case 'tmt-ui-dialog':
      return (
        <>
          <BrowserAction
            type="button"
            label={attribute(owner, 'trigger', attribute(owner, 'title'))}
            variant="secondary"
            onActivate={() => changed(owner, 'open', true)}
          />
          <BrowserDialog
            title={attribute(owner, 'title')}
            open={boolState(owner, 'open')}
            keepMounted
            size={attribute(owner, 'size', 'regular') as 'regular' | 'wide'}
            presentation={attribute(owner, 'presentation', 'standard') as 'standard' | 'confirm'}
            onDismiss={() => changed(owner, 'open', false)}
            actions={
              authoredSlot(owner, 'actions').length ? (
                <AuthoredSlot nodes={authoredSlot(owner, 'actions')} />
              ) : undefined
            }
          >
            {body()}
          </BrowserDialog>
        </>
      );
    case 'tmt-ui-chips': {
      const entries = items(owner);
      const stored = owner.state.get('selected');
      const selected = Array.isArray(stored)
        ? (stored as string[])
        : entries
            .filter((entry) => entry.node.hasAttribute('data-selected'))
            .map((entry) => entry.id);
      const query = stringState(owner, 'query', '');
      return (
        <BrowserChips
          label={label}
          disabled={disabled}
          items={entries
            .filter((entry) => selected.includes(entry.id))
            .map(({ id, label }) => ({ id, label, removeLabel: `Remove ${label}` }))}
          options={entries
            .filter(
              (entry) =>
                !selected.includes(entry.id) &&
                entry.label.toLowerCase().includes(query.toLowerCase()),
            )
            .map(({ id, label }) => ({ id, label }))}
          query={query}
          onQueryChange={(value) => changed(owner, 'query', value)}
          onAdd={(value) => {
            owner.state.set('query', '');
            changed(owner, 'selected', [...selected, value]);
          }}
          onRemove={(value) =>
            changed(
              owner,
              'selected',
              selected.filter((id) => id !== value),
            )
          }
          addLabel="Add tag"
          searchLabel="Find a tag"
          emptyText="No matching tags"
        />
      );
    }
    case 'tmt-ui-chip':
      return boolState(owner, 'removed') ? null : (
        <BrowserChip
          label={label}
          removeLabel={`Remove ${label}`}
          disabled={disabled}
          onRemove={() => changed(owner, 'removed', true)}
        />
      );
    case 'tmt-ui-notice': {
      const tone = attribute(owner, 'tone', 'muted') as BrowserNoticeTone;
      const states = {
        waiting: ['◆', 'Needs you'],
        blocked: ['✗', 'Blocked'],
        working: ['●', 'Working'],
        review: ['◐', 'In review'],
        muted: ['i', 'Information'],
      };
      const [mark, word] = states[tone];
      return (
        <BrowserNotice
          tone={tone}
          variant={attribute(owner, 'variant', 'neutral') as 'neutral' | 'tinted'}
          title={attribute(owner, 'title')}
          mark={mark}
          stateLabel={attribute(owner, 'state-label', word)}
          announcement="none"
          body={body()}
          actions={
            authoredSlot(owner, 'actions').length ? (
              <AuthoredSlot nodes={authoredSlot(owner, 'actions')} />
            ) : undefined
          }
        />
      );
    }
  }
  return null;
}
const registered = new WeakSet<CustomElementRegistry>();
/** Registers only presentation elements. The caller owns the document, CSS and runtime lifetime. */
export function registerBrowserElements(): void {
  const registry = customElements;
  if (registered.has(registry)) return;
  for (const definition of browserElements) {
    if (registry.get(definition.tag))
      throw new Error(`Element already registered: ${definition.tag}`);
  }
  for (const definition of browserElements) {
    registry.define(
      definition.tag,
      class extends HTMLElement {
        static observedAttributes = Object.keys(definition.attributes);
        private root?: Root;
        private owner?: ElementOwner;
        private waiting?: () => void;
        connectedCallback(): void {
          const initialize = () => {
            this.waiting = undefined;
            if (!this.isConnected || this.root) return;
            const source = [...this.childNodes];
            const text = source
              .map((node) => node.textContent ?? '')
              .join('')
              .trim();
            const slots = new Map(
              definition.slots.map((name) => [
                name,
                source.filter((node) =>
                  node instanceof Element
                    ? (node.getAttribute('slot') ?? 'body') === name
                    : name === 'body',
                ),
              ]),
            );
            const items = source
              .filter(
                (node): node is Element =>
                  node instanceof Element && node.getAttribute('slot') === definition.itemSlot,
              )
              .map((node) => ({
                id: node.getAttribute('data-value') ?? '',
                label: node.getAttribute('data-label') ?? node.textContent?.trim() ?? '',
                node,
                nodes: [node],
              }));
            const mount = this.ownerDocument.createElement('span');
            mount.style.display = 'contents';
            this.replaceChildren(mount);
            this.root = createRoot(mount);
            const owner: ElementOwner = {
              element: this,
              definition,
              source,
              slots,
              text,
              items,
              state: new Map(),
              render: () => {
                if (this.owner === owner) this.root?.render(<ElementView owner={owner} />);
              },
            };
            this.owner = owner;
            owner.render();
          };
          // A parser's connected callback can precede its children. The document boundary is
          // deterministic; dynamically inserted complete fragments only need the current turn.
          if (this.ownerDocument.readyState === 'loading') {
            if (this.waiting)
              this.ownerDocument.removeEventListener('DOMContentLoaded', this.waiting);
            this.waiting = initialize;
            this.ownerDocument.addEventListener('DOMContentLoaded', initialize, { once: true });
          } else queueMicrotask(initialize);
        }
        attributeChangedCallback(name: string): void {
          this.owner?.state.delete(name);
          this.owner?.render();
        }
        disconnectedCallback(): void {
          if (this.waiting) {
            this.ownerDocument.removeEventListener('DOMContentLoaded', this.waiting);
            this.waiting = undefined;
          }
          // Moving a slot within this document is not removal. A real disconnect owns cleanup.
          queueMicrotask(() => {
            if (this.isConnected) return;
            const source = this.owner?.source;
            this.root?.unmount();
            this.root = undefined;
            this.owner = undefined;
            if (source) this.replaceChildren(...source);
          });
        }
      },
    );
  }
  registered.add(registry);
}
