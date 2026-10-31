import { useId, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Pencil } from 'lucide-react';
import { registerBrowserElements } from '../../src/elements.js';
import { browserElements } from '../../src/elements-manifest.js';
import {
  BrowserAction,
  BrowserIconAction,
  BrowserField,
  BrowserToggle,
  BrowserCheckbox,
  BrowserMenu,
  BrowserTabs,
  BrowserDialog,
  BrowserChips,
  BrowserChip,
  BrowserNotice,
  BrowserListTable,
  BusyMark,
} from '../../src/react.js';
import type { BrowserActionVariant } from '../../src/static.js';
const query = new URLSearchParams(location.search);
const tag = query.get('tag');
const state = query.get('state') ?? 'default';
const definition = browserElements.find((entry) => entry.tag === tag);
const example = definition?.examples.find((entry) => entry.state === state);
if (!example) throw new Error('Unknown element fixture');
document.documentElement.dataset.theme = query.get('theme') ?? 'light';
const style = document.createElement('style');
style.textContent =
  '.element-table-reference{--tmt-ui-list-tracks:repeat(2,minmax(0,1fr));--tmt-ui-list-tracks-compact:minmax(0,1fr)}body{margin:0;background:var(--tmt-ui-surface-paper);color:var(--tmt-ui-color-text);font-family:var(--tmt-ui-font-body)}main{box-sizing:border-box;padding:var(--tmt-ui-space-l);width:100%}';
document.head.append(style);
registerBrowserElements();
registerBrowserElements();
document.querySelector('#custom')!.innerHTML = example.html;
function Reference() {
  const id = useId();
  const [pressed, press] = useState(state === 'on');
  const [checked, check] = useState(state === 'checked');
  const [mixed, mix] = useState(state === 'mixed');
  const [value, select] = useState('recent');
  const [tab, selectTab] = useState('all');
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('Design');
  const [text, setText] = useState(state === 'error' ? '' : 'Design');
  const [chips, setChips] = useState(['design']);
  const [search, setSearch] = useState('');
  const [removed, remove] = useState(false);
  switch (tag) {
    case 'tmt-ui-button':
      return (
        <BrowserAction
          type="button"
          label={state === 'busy' ? 'Saving' : 'Save'}
          variant={
            ['primary', 'secondary', 'text', 'ghost', 'destructive'].includes(state)
              ? (state as BrowserActionVariant)
              : 'secondary'
          }
          presentation={state === 'compact' ? 'compact' : undefined}
          busy={state === 'busy'}
          busyMark={state === 'busy' ? <BusyMark /> : undefined}
          disabled={state === 'disabled'}
          disabledReason={state === 'disabled' ? 'Read only' : undefined}
          disabledReasonId={`${id}-reason`}
          onActivate={() => {}}
        />
      );
    case 'tmt-ui-icon-button':
      return (
        <BrowserIconAction
          type="button"
          label="Edit"
          variant="secondary"
          icon={<Pencil />}
          disabled={state === 'disabled'}
          disabledReason={state === 'disabled' ? 'Read only' : undefined}
          disabledReasonId={`${id}-reason`}
          onActivate={() => {}}
        />
      );
    case 'tmt-ui-field':
      return (
        <BrowserField
          controlId={`${id}-input`}
          label="Name"
          error={state === 'error' ? 'Enter a name' : undefined}
          errorId={`${id}-error`}
          invalid={state === 'error'}
          renderControl={(props) => (
            <input
              {...props}
              placeholder=""
              disabled={false}
              value={text}
              onChange={(event) => setText(event.target.value)}
            />
          )}
        />
      );
    case 'tmt-ui-toggle':
      return (
        <BrowserToggle
          label="Auto-open pages"
          pressed={pressed}
          onActivate={() => press(!pressed)}
        />
      );
    case 'tmt-ui-checkbox':
      return (
        <label>
          <BrowserCheckbox
            aria-label="Include archived"
            checked={checked}
            indeterminate={mixed}
            onChange={(event) => {
              check(event.target.checked);
              mix(false);
            }}
          />{' '}
          Include archived
        </label>
      );
    case 'tmt-ui-menu':
      return (
        <BrowserMenu
          label="Sort"
          valueLabel={value === 'recent' ? 'Recently updated' : 'Title'}
          items={[
            {
              key: 'recent',
              label: 'Recently updated',
              checked: value === 'recent',
              role: 'menuitemradio',
              disabled: false,
            },
            {
              key: 'title',
              label: 'Title',
              checked: value === 'title',
              role: 'menuitemradio',
              disabled: false,
            },
          ]}
          onSelect={select}
        />
      );
    case 'tmt-ui-tabs':
      return (
        <BrowserTabs
          label="Tasks"
          tabs={[
            { id: 'all', label: 'All', tabId: `${id}-all-tab`, panelId: `${id}-all-panel` },
            { id: 'done', label: 'Done', tabId: `${id}-done-tab`, panelId: `${id}-done-panel` },
          ]}
          selected={tab}
          onSelect={selectTab}
        >
          {(item) => <section>{item.id === 'all' ? 'All tasks' : 'Completed tasks'}</section>}
        </BrowserTabs>
      );
    case 'tmt-ui-dialog':
      return (
        <>
          <BrowserAction
            type="button"
            label="Manage shortcuts"
            variant="secondary"
            onActivate={() => setOpen(true)}
          />
          <BrowserDialog
            title="Manage shortcuts"
            open={open}
            keepMounted
            onDismiss={() => setOpen(false)}
            actions={<p>Press Escape to close and keep the draft.</p>}
          >
            <div>
              <BrowserField
                controlId={`${id}-name`}
                label="Name"
                invalid={false}
                renderControl={(props) => (
                  <input
                    {...props}
                    placeholder=""
                    disabled={false}
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                  />
                )}
              />
            </div>
          </BrowserDialog>
        </>
      );
    case 'tmt-ui-chips': {
      const entries = [
        { id: 'design', label: 'Design' },
        { id: 'spec', label: 'Spec' },
      ];
      return (
        <BrowserChips
          label="Tags"
          items={entries
            .filter((entry) => chips.includes(entry.id))
            .map((entry) => ({ ...entry, removeLabel: `Remove ${entry.label}` }))}
          options={entries.filter(
            (entry) =>
              !chips.includes(entry.id) && entry.label.toLowerCase().includes(search.toLowerCase()),
          )}
          query={search}
          onQueryChange={setSearch}
          onAdd={(id) => {
            setChips([...chips, id]);
            setSearch('');
          }}
          onRemove={(id) => setChips(chips.filter((item) => item !== id))}
          addLabel="Add tag"
          searchLabel="Find a tag"
          emptyText="No matching tags"
        />
      );
    }
    case 'tmt-ui-chip':
      return removed ? null : (
        <BrowserChip
          label="Design"
          removeLabel="Remove Design"
          disabled={false}
          onRemove={() => remove(true)}
        />
      );
    case 'tmt-ui-list-table':
      return (
        <BrowserListTable
          label="Pages"
          className="element-table-reference"
          columns={[
            { id: 'title', label: 'Page' },
            { id: 'state', label: 'State' },
          ]}
          rows={
            state === 'empty'
              ? []
              : [
                  { id: 'notes', cells: ['Project notes', 'Draft'] },
                  { id: 'review', cells: ['Review', 'Ready'] },
                ]
          }
        />
      );
    case 'tmt-ui-notice':
      return (
        <BrowserNotice
          title="Two answers needed"
          tone="waiting"
          variant="neutral"
          mark="◆"
          stateLabel="Needs you"
          announcement="none"
          body={<p>Please review the open questions.</p>}
        />
      );
  }
  return null;
}
createRoot(document.querySelector('#reference')!).render(<Reference />);
