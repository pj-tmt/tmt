/** Product-neutral author contract. Runtime, checker and catalog consume this same data. */
export interface ElementAttribute {
  kind: 'text' | 'boolean' | 'enum' | 'json';
  values?: readonly string[];
  required?: boolean;
}
export interface ElementExample {
  state: string;
  html: string;
}
export interface BrowserElementDefinition {
  tag: string;
  name: string;
  component: string;
  category: 'Input' | 'Navigation' | 'Overlay' | 'Feedback' | 'Data';
  purpose: string;
  attributes: Readonly<Record<string, ElementAttribute>>;
  slots: readonly string[];
  itemSlot?: string;
  minimumItems?: number;
  textLabel?: boolean;
  tableData?: boolean;
  styles?: string;
  tokens: readonly string[];
  source: string;
  docs: string;
  gallery: string;
  examples: readonly ElementExample[];
}
const text: ElementAttribute = { kind: 'text' };
const required: ElementAttribute = { kind: 'text', required: true };
const boolean: ElementAttribute = { kind: 'boolean' };
const enumeration = (...values: string[]): ElementAttribute => ({ kind: 'enum', values });
export const elementIconNames = [
  'check',
  'plus',
  'pencil',
  'trash-2',
  'ellipsis',
  'settings',
  'info',
  'x',
  'link',
] as const;
const action = {
  label: text,
  variant: enumeration('primary', 'secondary', 'text', 'ghost', 'destructive'),
  presentation: enumeration('compact'),
  busy: boolean,
  disabled: boolean,
  'disabled-reason': text,
  icon: enumeration(...elementIconNames),
};

const definition = (
  value: Omit<BrowserElementDefinition, 'docs' | 'tokens'> &
    Partial<Pick<BrowserElementDefinition, 'docs' | 'tokens'>>,
): BrowserElementDefinition => ({
  docs: 'design/browser-ui/README.md#controlled-input-contracts',
  tokens: ['--tmt-ui-font-body', '--tmt-ui-color-text', '--tmt-ui-control-size'],
  ...value,
});
export const browserElements: readonly BrowserElementDefinition[] = [
  definition({
    tag: 'tmt-ui-button',
    name: 'Action',
    component: 'BrowserAction',
    category: 'Input',
    purpose: 'An action with the shared variants, busy timing and disabled explanation.',
    attributes: action,
    slots: [],
    textLabel: true,
    source: 'action.tsx',
    gallery: 'actions',
    examples: [
      ...['primary', 'secondary', 'text', 'ghost', 'destructive'].map((variant) => ({
        state: variant,
        html: `<tmt-ui-button variant="${variant}">Save</tmt-ui-button>`,
      })),
      { state: 'busy', html: '<tmt-ui-button busy>Saving</tmt-ui-button>' },
      {
        state: 'disabled',
        html: '<tmt-ui-button disabled disabled-reason="Read only">Save</tmt-ui-button>',
      },
      { state: 'compact', html: '<tmt-ui-button presentation="compact">Save</tmt-ui-button>' },
    ],
  }),
  definition({
    tag: 'tmt-ui-icon-button',
    name: 'Icon action',
    component: 'BrowserIconAction',
    category: 'Input',
    purpose: 'A named icon action with keyboard and pointer help.',
    attributes: {
      ...action,
      label: required,
      icon: enumeration(...elementIconNames),
      variant: enumeration('primary', 'secondary', 'text', 'destructive'),
    },
    slots: [],
    source: 'icon-action.tsx',
    gallery: 'icon-actions',
    examples: [
      {
        state: 'default',
        html: '<tmt-ui-icon-button label="Edit" icon="pencil"></tmt-ui-icon-button>',
      },
      {
        state: 'disabled',
        html: '<tmt-ui-icon-button label="Edit" disabled disabled-reason="Read only"></tmt-ui-icon-button>',
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-field',
    name: 'Field',
    component: 'BrowserField',
    category: 'Input',
    purpose: 'A real text input with the shared label, description and error relationships.',
    attributes: {
      label: required,
      value: text,
      placeholder: text,
      description: text,
      error: text,
      disabled: boolean,
    },
    slots: [],
    source: 'field.tsx',
    gallery: 'fields',
    examples: [
      { state: 'default', html: '<tmt-ui-field label="Name" value="Design"></tmt-ui-field>' },
      { state: 'error', html: '<tmt-ui-field label="Name" error="Enter a name"></tmt-ui-field>' },
    ],
  }),
  definition({
    tag: 'tmt-ui-toggle',
    name: 'Toggle',
    component: 'BrowserToggle',
    category: 'Input',
    purpose: 'A local on/off choice.',
    attributes: { label: required, pressed: boolean, disabled: boolean },
    slots: [],
    source: 'toggle.tsx',
    gallery: 'toggle',
    examples: [
      { state: 'off', html: '<tmt-ui-toggle label="Auto-open pages"></tmt-ui-toggle>' },
      { state: 'on', html: '<tmt-ui-toggle label="Auto-open pages" pressed></tmt-ui-toggle>' },
    ],
  }),
  definition({
    tag: 'tmt-ui-checkbox',
    name: 'Checkbox',
    component: 'BrowserCheckbox',
    category: 'Input',
    purpose: 'A labelled native checkbox with the shared drawing.',
    attributes: { label: required, checked: boolean, indeterminate: boolean, disabled: boolean },
    slots: [],
    source: 'checkbox.tsx',
    gallery: 'checkbox',
    examples: [
      { state: 'unchecked', html: '<tmt-ui-checkbox label="Include archived"></tmt-ui-checkbox>' },
      {
        state: 'checked',
        html: '<tmt-ui-checkbox label="Include archived" checked></tmt-ui-checkbox>',
      },
      {
        state: 'mixed',
        html: '<tmt-ui-checkbox label="Include archived" indeterminate></tmt-ui-checkbox>',
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-menu',
    name: 'Menu',
    component: 'BrowserMenu',
    category: 'Navigation',
    purpose: 'A keyboard-operable local choice menu.',
    attributes: {
      label: required,
      value: text,
      disabled: boolean,
      presentation: enumeration('compact'),
    },
    slots: ['item'],
    itemSlot: 'item',
    minimumItems: 1,
    source: 'menu.tsx',
    gallery: 'menus',
    docs: 'design/browser-ui/README.md#drawer-and-menu',
    examples: [
      {
        state: 'default',
        html: '<tmt-ui-menu label="Sort" value="recent"><span slot="item" data-value="recent">Recently updated</span><span slot="item" data-value="title">Title</span></tmt-ui-menu>',
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-tabs',
    name: 'Tabs',
    component: 'BrowserTabs',
    category: 'Navigation',
    purpose: 'Local tab selection with panels kept mounted.',
    attributes: {
      label: required,
      value: text,
      variant: enumeration('boxed', 'flush', 'flat'),
      wrap: boolean,
    },
    slots: ['panel'],
    itemSlot: 'panel',
    minimumItems: 1,
    source: 'tabs.tsx',
    gallery: 'tabs',
    examples: [
      {
        state: 'default',
        html: '<tmt-ui-tabs label="Tasks"><section slot="panel" data-value="all" data-label="All">All tasks</section><section slot="panel" data-value="done" data-label="Done">Completed tasks</section></tmt-ui-tabs>',
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-dialog',
    name: 'Dialog',
    component: 'BrowserDialog',
    category: 'Overlay',
    purpose: 'A local modal with its real focus and dismissal behavior.',
    attributes: {
      title: required,
      trigger: text,
      size: enumeration('regular', 'wide'),
      presentation: enumeration('standard', 'confirm'),
    },
    slots: ['body', 'actions'],
    source: 'dialog.tsx',
    gallery: 'overlays',
    docs: 'design/browser-ui/README.md#dialog',
    examples: [
      {
        state: 'default',
        html: '<tmt-ui-dialog title="Manage shortcuts" trigger="Manage shortcuts"><div slot="body"><tmt-ui-field label="Name" value="Design"></tmt-ui-field></div><p slot="actions">Press Escape to close and keep the draft.</p></tmt-ui-dialog>',
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-chips',
    name: 'Chips',
    component: 'BrowserChips',
    category: 'Input',
    purpose: 'A local searchable choice of removable chips.',
    attributes: { label: required, disabled: boolean },
    slots: ['option'],
    itemSlot: 'option',
    minimumItems: 0,
    source: 'chips.tsx',
    gallery: 'chips',
    docs: 'design/browser-ui/README.md#chips',
    examples: [
      {
        state: 'default',
        html: '<tmt-ui-chips label="Tags"><span slot="option" data-value="design" data-selected>Design</span><span slot="option" data-value="spec">Spec</span></tmt-ui-chips>',
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-chip',
    name: 'Chip',
    component: 'BrowserChip',
    category: 'Input',
    purpose: 'A removable chip; removal affects only this preview.',
    attributes: { label: text, disabled: boolean },
    slots: [],
    textLabel: true,
    source: 'chips.tsx',
    gallery: 'chips',
    docs: 'design/browser-ui/README.md#chips',
    examples: [{ state: 'default', html: '<tmt-ui-chip>Design</tmt-ui-chip>' }],
  }),
  definition({
    tag: 'tmt-ui-list-table',
    name: 'List table',
    component: 'BrowserListTable',
    category: 'Data',
    purpose:
      'Text rows in the shared responsive list table; the author supplies the track variables.',
    attributes: {
      label: required,
      columns: { kind: 'json', required: true },
      rows: { kind: 'json', required: true },
    },
    slots: [],
    tableData: true,
    source: 'list-table.tsx',
    styles: 'list.css',
    gallery: 'list-table',
    docs: 'design/browser-ui/README.md#list-table-and-page-subheader',
    examples: [
      {
        state: 'default',
        html: `<tmt-ui-list-table label="Pages" columns='["Page","State"]' rows='[["Project notes","Draft"],["Review","Ready"]]' style="--tmt-ui-list-tracks:repeat(2,minmax(0,1fr));--tmt-ui-list-tracks-compact:minmax(0,1fr)"></tmt-ui-list-table>`,
      },
      {
        state: 'empty',
        html: `<tmt-ui-list-table label="Pages" columns='["Page","State"]' rows='[]' style="--tmt-ui-list-tracks:repeat(2,minmax(0,1fr));--tmt-ui-list-tracks-compact:minmax(0,1fr)"></tmt-ui-list-table>`,
      },
    ],
  }),
  definition({
    tag: 'tmt-ui-notice',
    name: 'Notice',
    component: 'BrowserNotice',
    category: 'Feedback',
    purpose: 'A shared notice with a state word and mark.',
    attributes: {
      title: required,
      tone: enumeration('waiting', 'blocked', 'working', 'review', 'muted'),
      variant: enumeration('neutral', 'tinted'),
      'state-label': text,
    },
    slots: ['body', 'actions'],
    source: 'notice.tsx',
    gallery: 'notice',
    examples: [
      {
        state: 'default',
        html: '<tmt-ui-notice title="Two answers needed" tone="waiting"><p slot="body">Please review the open questions.</p></tmt-ui-notice>',
      },
    ],
  }),
];

/** The author table accepts text data, never HTML or host row commands. */
export function listTableData(
  columnsSource: string,
  rowsSource: string,
): { columns: readonly string[]; rows: readonly (readonly string[])[] } | undefined {
  try {
    const columns: unknown = JSON.parse(columnsSource);
    const rows: unknown = JSON.parse(rowsSource);
    if (
      !Array.isArray(columns) ||
      !columns.length ||
      !columns.every((value) => typeof value === 'string' && value.trim())
    )
      return;
    if (
      !Array.isArray(rows) ||
      !rows.every(
        (row) =>
          Array.isArray(row) &&
          row.length === columns.length &&
          row.every((value) => typeof value === 'string'),
      )
    )
      return;
    return { columns: columns as string[], rows: rows as string[][] };
  } catch {
    return;
  }
}

export function browserElement(name: string): BrowserElementDefinition | undefined {
  return browserElements.find((entry) => entry.tag === name);
}
/** Only explicitly supported attributes belong to the widget API; ordinary HTML metadata remains author-owned. */
export function elementAttributeIssue(
  definition: BrowserElementDefinition,
  name: string,
  value: string,
): string | undefined {
  const rule = definition.attributes[name];
  if (!rule)
    return /^(?:id|class|style|title|lang|dir|hidden|slot|aria-[a-z-]+|data-[a-z0-9-]+)$/.test(name)
      ? undefined
      : 'unknown-ui-attribute';
  if (rule.required && !value.trim()) return 'invalid-ui-attribute';
  if (rule.kind === 'json') {
    try {
      JSON.parse(value);
    } catch {
      return 'invalid-ui-attribute';
    }
  }
  if (rule.kind === 'enum' && !rule.values?.includes(value)) return 'invalid-ui-attribute';
  if (rule.kind === 'boolean' && !['', name].includes(value)) return 'invalid-ui-attribute';
  return undefined;
}

/** Conservative mount-key hint, never markup admission. A false match only remounts a page. */
export function mayUseBrowserElements(source: string): boolean {
  const lower = source.toLowerCase();
  return browserElements.some(({ tag }) => lower.includes(`<${tag}`));
}
