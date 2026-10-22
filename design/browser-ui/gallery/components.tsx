import { useEffect, useState, type ReactNode } from 'react';
import { Copy, Plus, Settings, Trash2 } from 'lucide-react';
import {
  BrandLockup,
  BrandMark,
  BrowserAction,
  BrowserChip,
  BrowserCheckbox,
  BrowserChips,
  BrowserCommand,
  BrowserField,
  BrowserHeader,
  BrowserIconAction,
  BrowserList,
  BrowserListRow,
  BrowserMenu,
  BrowserNotice,
  BrowserTabs,
  BrowserToggle,
  busyMarkDelayMs,
  busyMarkHoldMs,
  useBusyMark,
  type BrowserChipItem,
  type BrowserMenuItem,
  type BrowserNoticeTone,
  type BrowserTab,
  BusyMark,
} from '../src/react.js';
import { Cell, Cells, FramePair, Pair, Section, noop, type Theme } from './kit.js';

const variants = ['primary', 'text', 'destructive'] as const;

export function BrandSection() {
  const sizes = [16, 20, 24, 32, 48, 64, 96];
  return (
    <Section id="brand">
      <Pair
        title="BrandMark at every size"
        note="Below 28px the simplified small form renders. Aurora colours appear only here."
      >
        {() => (
          <div className="gallery-row gallery-row-end">
            {sizes.map((size) => (
              <Cell key={size} label={`${size}`}>
                <BrandMark size={size} />
              </Cell>
            ))}
          </div>
        )}
      </Pair>
      <Pair title="BrandLockup">
        {() => (
          <div className="gallery-row gallery-row-end">
            {[24, 32, 48].map((size) => (
              <Cell key={size} label={`${size}`}>
                <BrandLockup size={size} />
              </Cell>
            ))}
          </div>
        )}
      </Pair>
    </Section>
  );
}

export function HeaderSection() {
  return (
    <Section id="header">
      <Pair title="BrowserHeader">
        {(theme) => (
          <div className="gallery-stack">
            <BrowserHeader productLabel="Colab" title="Pages" />
            <BrowserHeader
              productLabel="Colab"
              title="Release plan for the remote channel"
              caption="Updated 2 minutes ago"
              captionId={`header-caption-${theme}`}
              headingSuffix={<span>· draft</span>}
              status={<span>◆ 2 need you</span>}
              actions={
                <>
                  <BrowserIconAction
                    type="button"
                    variant="text"
                    label="Settings"
                    icon={<Settings />}
                    onActivate={noop}
                  />
                  <BrowserAction type="button" variant="primary" label="Share" onActivate={noop} />
                </>
              }
            />
          </div>
        )}
      </Pair>
    </Section>
  );
}

function stateCells(render: (state: string) => ReactNode) {
  return (
    <Cells>
      <Cell label="idle">{render('idle')}</Cell>
      <Cell label="hover" pin="hover">
        {render('hover')}
      </Cell>
      <Cell label="focus" pin="focus">
        {render('focus')}
      </Cell>
      <Cell label="disabled">{render('disabled')}</Cell>
      <Cell label="busy">{render('busy')}</Cell>
    </Cells>
  );
}

export function ActionsSection() {
  return (
    <Section id="actions">
      {variants.map((variant) => (
        <Pair key={variant} title={`BrowserAction · ${variant}`}>
          {(theme) =>
            stateCells((state) => (
              <BrowserAction
                type="button"
                variant={variant}
                label={variant === 'destructive' ? 'Delete page' : 'Save draft'}
                disabled={state === 'disabled'}
                disabledReason={state === 'disabled' ? 'Read only' : undefined}
                disabledReasonId={`action-${variant}-${theme}-reason`}
                busy={state === 'busy'}
                busyMark={<BusyMark />}
                onActivate={noop}
              />
            ))
          }
        </Pair>
      ))}
    </Section>
  );
}

export function IconActionsSection() {
  return (
    <Section id="icon-actions">
      {variants.map((variant) => (
        <Pair key={variant} title={`BrowserIconAction · ${variant}`}>
          {() =>
            stateCells((state) => (
              <BrowserIconAction
                type="button"
                variant={variant}
                label={variant === 'destructive' ? 'Delete' : 'Add'}
                icon={variant === 'destructive' ? <Trash2 /> : <Plus />}
                disabled={state === 'disabled'}
                busy={state === 'busy'}
                busyMark={<BusyMark />}
                onActivate={noop}
              />
            ))
          }
        </Pair>
      ))}
      <Pair title="Pressed and expanded">
        {(theme) => (
          <Cells>
            <Cell label="pressed">
              <BrowserIconAction
                type="button"
                variant="text"
                label="Pin"
                icon={<Plus />}
                pressed
                onActivate={noop}
              />
            </Cell>
            <Cell label="not pressed">
              <BrowserIconAction
                type="button"
                variant="text"
                label="Pin"
                icon={<Plus />}
                pressed={false}
                onActivate={noop}
              />
            </Cell>
            <Cell label="expanded">
              <BrowserIconAction
                type="button"
                variant="text"
                label="Settings"
                icon={<Settings />}
                expanded
                controls={`icon-panel-${theme}`}
                onActivate={noop}
              />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

function BusyDemo() {
  const [busy, setBusy] = useState(true);
  const shown = useBusyMark(busy);
  useEffect(() => {
    const timer = setInterval(() => setBusy((value) => !value), 2000);
    return () => clearInterval(timer);
  }, []);
  return (
    <p className="gallery-mono">
      useBusyMark: busy <b>{String(busy)}</b> · mark shown <b>{String(shown)}</b>
    </p>
  );
}

export function BusySection() {
  return (
    <Section id="busy">
      <Pair
        title="useBusyMark"
        note={`A mark appears only after ${busyMarkDelayMs} ms of busy and then holds at least ${busyMarkHoldMs} ms. The demo flips busy every 2 s.`}
      >
        {() => <BusyDemo />}
      </Pair>
      <Pair
        title="BusyMark"
        note="The one mark (a 2x3 braille snake), alone at 12, 16 and 32 px; the busy state of each action and icon action is in their sections. It steps once per --tmt-ui-spin-duration and holds its first frame under reduced motion."
      >
        {() => (
          <Cells>
            {[12, 16, 32].map((size) => (
              <Cell key={size} label={`${size}px`}>
                <span className="gallery-busy-size" style={{ fontSize: size }}>
                  <BusyMark />
                </span>
              </Cell>
            ))}
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

export function FieldsSection() {
  return (
    <Section id="fields">
      <Pair title="BrowserField">
        {(theme) => (
          <Cells>
            <Cell label="idle">
              <BrowserField
                controlId={`field-idle-${theme}`}
                label="Page title"
                renderControl={(props) => <input {...props} defaultValue="Release plan" />}
              />
            </Cell>
            <Cell label="hover" pin="hover">
              <BrowserField
                controlId={`field-hover-${theme}`}
                label="Page title"
                renderControl={(props) => <input {...props} defaultValue="Release plan" />}
              />
            </Cell>
            <Cell label="focus" pin="focus">
              <BrowserField
                controlId={`field-focus-${theme}`}
                label="Page title"
                renderControl={(props) => <input {...props} defaultValue="Release plan" />}
              />
            </Cell>
            <Cell label="description">
              <BrowserField
                controlId={`field-description-${theme}`}
                label="Remote address"
                description="Shown to paired browsers only."
                descriptionId={`field-description-${theme}-text`}
                renderControl={(props) => <input {...props} placeholder="https://" />}
              />
            </Cell>
            <Cell label="invalid">
              <BrowserField
                controlId={`field-invalid-${theme}`}
                label="Remote address"
                invalid
                error="Use an https address."
                errorId={`field-invalid-${theme}-error`}
                renderControl={(props) => <input {...props} defaultValue="http://x" />}
              />
            </Cell>
            <Cell label="disabled">
              <BrowserField
                controlId={`field-disabled-${theme}`}
                label="Owner"
                renderControl={(props) => <input {...props} disabled defaultValue="ben" />}
              />
            </Cell>
            <Cell label="textarea">
              <BrowserField
                controlId={`field-textarea-${theme}`}
                label="Comment"
                renderControl={(props) => (
                  <textarea {...props} rows={3} defaultValue={'Two lines\nof text'} />
                )}
              />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

function ToggleDemo({ initial, disabled }: { initial: boolean; disabled?: boolean }) {
  const [pressed, setPressed] = useState(initial);
  return (
    <BrowserToggle
      pressed={pressed}
      label="Notify me"
      disabled={disabled}
      onActivate={() => setPressed(!pressed)}
    />
  );
}

export function ToggleSection() {
  return (
    <Section id="toggle">
      <Pair title="BrowserToggle">
        {() => (
          <Cells>
            <Cell label="off">
              <ToggleDemo initial={false} />
            </Cell>
            <Cell label="on">
              <ToggleDemo initial />
            </Cell>
            <Cell label="hover" pin="hover">
              <ToggleDemo initial={false} />
            </Cell>
            <Cell label="focus" pin="focus">
              <ToggleDemo initial />
            </Cell>
            <Cell label="disabled">
              <ToggleDemo initial disabled />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

const agents = [
  { id: 'core', label: 'core-ops', description: 'Core squad' },
  { id: 'ux', label: 'ux-4', description: 'UX squad' },
  { id: 'remote', label: 'remote-1', description: 'Remote squad' },
  { id: 'infra', label: 'infra-1', description: 'Busy', disabled: true },
];

function ChipsDemo({ disabled }: { disabled?: boolean }) {
  const [ids, setIds] = useState(['core']);
  const [query, setQuery] = useState('');
  const items: BrowserChipItem[] = ids.map((id) => {
    const agent = agents.find((entry) => entry.id === id)!;
    return { id, label: agent.label, removeLabel: `Remove ${agent.label}` };
  });
  return (
    <BrowserChips
      label="Agents"
      items={items}
      options={agents.filter(
        (agent) => !ids.includes(agent.id) && agent.label.includes(query.toLowerCase()),
      )}
      query={query}
      onQueryChange={setQuery}
      onAdd={(id) => setIds([...ids, id])}
      onRemove={(id) => setIds(ids.filter((entry) => entry !== id))}
      addLabel="Add agent"
      searchLabel="Search agents"
      emptyText="No matching agents"
      maxItems={3}
      limitText="Three agents at most"
      disabled={disabled}
    />
  );
}

export function ChipsSection() {
  return (
    <Section id="chips">
      <Pair title="BrowserChip">
        {() =>
          stateCells((state) =>
            state === 'busy' ? (
              <span className="gallery-note">no busy state</span>
            ) : (
              <BrowserChip
                label="core-ops"
                removeLabel="Remove core-ops"
                disabled={state === 'disabled'}
                onRemove={noop}
              />
            ),
          )
        }
      </Pair>
      <Pair title="BrowserChips" note="Live: press ＋ to open the real add popup.">
        {() => (
          <Cells>
            <Cell label="idle">
              <ChipsDemo />
            </Cell>
            <Cell label="disabled">
              <ChipsDemo disabled />
            </Cell>
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

export const menuItems: BrowserMenuItem[] = [
  { key: 'rename', label: 'Rename' },
  { key: 'move', label: 'Move to project' },
  { key: 'archive', label: 'Archive', disabled: true, disabledReason: 'Read only' },
  { key: 'delete', label: 'Delete', separated: true },
];

export function MenusSection() {
  const [sort, setSort] = useState('Updated');
  const sortItems: BrowserMenuItem[] = ['Updated', 'Created', 'Title'].map((key) => ({
    key,
    label: key,
    role: 'menuitemradio',
    checked: key === sort,
  }));
  return (
    <Section id="menus">
      <Pair title="BrowserMenu triggers">
        {() => (
          <Cells>
            <Cell label="icon">
              <BrowserMenu label="Page actions" items={menuItems} onSelect={noop} />
            </Cell>
            <Cell label="icon hover" pin="hover">
              <BrowserMenu label="Page actions" items={menuItems} onSelect={noop} />
            </Cell>
            <Cell label="icon focus" pin="focus">
              <BrowserMenu label="Page actions" items={menuItems} onSelect={noop} />
            </Cell>
            <Cell label="icon disabled">
              <BrowserMenu label="Page actions" items={menuItems} onSelect={noop} disabled />
            </Cell>
            <Cell label="text">
              <BrowserMenu label="New" triggerText="New" items={menuItems} onSelect={noop} />
            </Cell>
            <Cell label="field">
              <BrowserMenu
                label="Sort"
                valueLabel={sort}
                items={sortItems}
                onSelect={(key) => setSort(key)}
              />
            </Cell>
            <Cell label="field focus" pin="focus">
              <BrowserMenu label="Sort" valueLabel={sort} items={sortItems} onSelect={noop} />
            </Cell>
            <Cell label="select (static page markup)">
              <button type="button" className="tmt-ui-menu-trigger" data-variant="select">
                <span className="tmt-ui-menu-value">Ask first</span>
              </button>
            </Cell>
            <Cell label="select focus" pin="focus">
              <button type="button" className="tmt-ui-menu-trigger" data-variant="select">
                <span className="tmt-ui-menu-value">Ask first</span>
              </button>
            </Cell>
            <Cell label="select disabled">
              <button type="button" className="tmt-ui-menu-trigger" data-variant="select" disabled>
                <span className="tmt-ui-menu-value">Ask first</span>
              </button>
            </Cell>
            <Cell label="crumb">
              <BrowserMenu
                label="Project"
                valueLabel="Release planning"
                variant="crumb"
                items={sortItems}
                onSelect={noop}
              />
            </Cell>
          </Cells>
        )}
      </Pair>
      <FramePair title="BrowserMenu open" frame="menu" height={300} />
    </Section>
  );
}

const tabs: BrowserTab[] = [
  { id: 'layout', tabId: 'layout', panelId: 'layout', label: 'Layout' },
  {
    id: 'pages',
    tabId: 'pages',
    panelId: 'pages',
    label: 'Pages',
    state: { mark: '◆', word: 'Waiting', tone: 'waiting' },
  },
  {
    id: 'done',
    tabId: 'done',
    panelId: 'done',
    label: 'Answered',
    state: { mark: '✓', word: 'Answered', tone: 'working' },
  },
  {
    id: 'blocked',
    tabId: 'blocked',
    panelId: 'blocked',
    label: 'Blocked',
    state: { mark: '✗', word: 'Blocked', tone: 'blocked' },
  },
];

function TabsDemo({ theme, variant }: { theme: Theme; variant: 'boxed' | 'flush' }) {
  const [selected, setSelected] = useState('pages');
  const scoped = tabs.map((tab) => ({
    ...tab,
    tabId: `${variant}-${theme}-${tab.tabId}-tab`,
    panelId: `${variant}-${theme}-${tab.panelId}-panel`,
  }));
  return (
    <BrowserTabs
      label={`Decisions ${variant}`}
      tabs={scoped}
      selected={selected}
      status="1/4 answered"
      variant={variant}
      onSelect={(id) => setSelected(id)}
    >
      {(tab) => <p className="gallery-panel">{tab.label} panel</p>}
    </BrowserTabs>
  );
}

export function TabsSection() {
  return (
    <Section id="tabs">
      <Pair title="BrowserTabs · boxed">
        {(theme) => <TabsDemo theme={theme} variant="boxed" />}
      </Pair>
      <Pair title="BrowserTabs · flush">
        {(theme) => <TabsDemo theme={theme} variant="flush" />}
      </Pair>
    </Section>
  );
}

export function ListSection() {
  return (
    <Section id="list">
      <Pair title="BrowserList and BrowserListRow">
        {() => (
          <BrowserList label="Pages">
            <BrowserListRow
              title={<a href="#list">Release plan for the remote channel</a>}
              metadata="ux-4 · 2 minutes ago"
              state={<span>◆ needs you</span>}
              actions={<BrowserMenu label="Row actions" items={menuItems} onSelect={noop} />}
            />
            <BrowserListRow
              leading={<input type="checkbox" aria-label="Select brand sheet" defaultChecked />}
              title={<a href="#list">Brand sheet</a>}
              metadata="tmt-ux-lead · yesterday"
              state={<span>✓ answered</span>}
            />
          </BrowserList>
        )}
      </Pair>
      <Pair title="Row title link states">
        {() => (
          <Cells>
            {(['hover', 'focus'] as const).map((pin) => (
              <Cell key={pin} label={pin} pin={pin}>
                <BrowserList label={`Pinned ${pin}`}>
                  <BrowserListRow
                    title={<a href="#list">Brand sheet</a>}
                    metadata="tmt-ux-lead · yesterday"
                    state={<span>○ idle</span>}
                  />
                </BrowserList>
              </Cell>
            ))}
          </Cells>
        )}
      </Pair>
    </Section>
  );
}

function CommandDemo() {
  const [copied, setCopied] = useState(false);
  return (
    <BrowserCommand
      text="tmt upgrade --channel alpha --yes"
      action={
        <BrowserIconAction
          type="button"
          variant="text"
          label="Copy command"
          icon={<Copy />}
          onActivate={() => setCopied(true)}
        />
      }
      feedback={copied ? 'Copied' : undefined}
    />
  );
}

export function CommandSection() {
  return (
    <Section id="command">
      <Pair title="BrowserCommand">
        {() => (
          <div className="gallery-stack">
            <CommandDemo />
            <BrowserCommand text="tmt colab open" feedback="Copied" />
          </div>
        )}
      </Pair>
    </Section>
  );
}

const tones: [BrowserNoticeTone, string, string][] = [
  ['waiting', '◆', 'Waiting'],
  ['working', '●', 'Working'],
  ['review', '◐', 'Review'],
  ['blocked', '✗', 'Blocked'],
  ['muted', '○', 'Idle'],
];

export function NoticeSection() {
  return (
    <Section id="notice">
      <Pair title="BrowserNotice tones">
        {() => (
          <div className="gallery-stack">
            {tones.map(([tone, mark, word]) => (
              <BrowserNotice
                key={tone}
                tone={tone}
                announcement="none"
                mark={mark}
                stateLabel={word}
                eyebrow="Colab"
                title={`${word}: this page`}
                body="The owner's browser shows this notice with the explicit words for its state."
                actions={
                  tone === 'blocked' ? (
                    <BrowserAction
                      type="button"
                      variant="primary"
                      label="Retry"
                      onActivate={noop}
                    />
                  ) : undefined
                }
              />
            ))}
          </div>
        )}
      </Pair>
    </Section>
  );
}

export function CheckboxSection() {
  return (
    <Section id="checkbox">
      <Pair title="BrowserCheckbox (native input, class tmt-ui-checkbox)">
        {() => (
          <Cells>
            {(
              [
                ['idle', {}],
                ['checked', { defaultChecked: true }],
                ['indeterminate', { indeterminate: true }],
                ['hover', { pin: 'hover' }],
                ['focus', { defaultChecked: true, pin: 'focus' }],
                ['disabled', { disabled: true }],
                ['disabled checked', { disabled: true, defaultChecked: true }],
              ] as const
            ).map(([label, { pin, ...props }]: readonly [string, CheckboxState]) => (
              <Cell key={label} label={label} pin={pin}>
                <label className="tmt-ui-checkbox-label">
                  <BrowserCheckbox {...props} />
                  Notify me
                </label>
              </Cell>
            ))}
          </Cells>
        )}
      </Pair>
    </Section>
  );
}
type CheckboxState = {
  defaultChecked?: boolean;
  indeterminate?: boolean;
  disabled?: boolean;
  pin?: 'hover' | 'focus';
};
