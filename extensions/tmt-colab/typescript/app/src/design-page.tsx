import { useEffect, useMemo, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from '@tanstack/react-router';
import { BrowserPageSubheader, BrowserTable, BrowserTabs } from '@tmt/browser-ui/react';
import { browserUiClasses as ui } from '@tmt/browser-ui/static';
import { browserCatalog, browserPresentationInventory } from '@tmt/browser-ui/catalog';
import { designSystems } from './design-systems.js';
import { designDocument } from './design-document.js';
import { mountRenderer, type RenderState } from './renderer.js';
import { CopyCommand } from './copy-command.js';
import './design-page.css';

type CatalogEntry = (typeof browserCatalog)[number];
const PREVIEW_HEIGHT = 72;
function Preview({
  html,
  label,
  document = false,
}: {
  html: string;
  label: string;
  document?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<RenderState>();
  const source = useMemo(
    () =>
      `<style>body{margin:var(--tmt-ui-space-m);font:15px/1.5 var(--tmt-ui-font-body)}body>section{padding:var(--tmt-ui-space-m);background:var(--tmt-ui-surface-paper);color:var(--tmt-ui-color-text)}</style>${html}`,
    [html],
  );
  useEffect(() => {
    const controller = new AbortController();
    setState(undefined);
    void mountRenderer(ref.current!, source, {
      signal: controller.signal,
      onState: setState,
      minimumHeight: PREVIEW_HEIGHT,
      ...(document ? { layout: 'column' as const } : {}),
    })
      .then((renderer) => {
        if (controller.signal.aborted) renderer.destroy();
      })
      .catch(() => {
        if (!controller.signal.aborted) setState('failed');
      });
    return () => controller.abort();
  }, [source, document]);
  return (
    <section
      className={document ? 'design-document' : 'design-preview'}
      aria-label={label}
      data-design-ready={state === 'ready' ? '' : undefined}
    >
      <div ref={ref} />
      {state === 'failed' && <p role="alert">This preview could not load. Reload to try again.</p>}
    </section>
  );
}
function Coverage({ entry }: { entry: CatalogEntry }) {
  return (
    <p className="design-coverage" aria-label="Indexed coverage">
      {(['docs', 'gallery', 'test'] as const).map((key) => (
        <span
          key={key}
          className={ui.countChip}
          data-tone={entry.coverage[key] ? 'neutral' : 'attention'}
          data-covered={entry.coverage[key]}
          title={
            key === 'test'
              ? 'Discovered parity tests; CI results are separate.'
              : `${key} entry found in the build inventory.`
          }
        >
          {entry.coverage[key] ? '✓' : '—'}{' '}
          {key === 'docs' ? 'Docs' : key === 'gallery' ? 'Gallery' : 'Test'}
        </span>
      ))}
    </p>
  );
}
function Detail({ entry }: { entry: CatalogEntry }) {
  const states = ['light', 'dark']
    .map(
      (theme) =>
        `<section data-theme="${theme}"><h2>States · ${theme}</h2>${entry.examples.map((example) => `<section><h3>${example.state}</h3>${example.html}</section>`).join('')}</section>`,
    )
    .join('');
  return (
    <>
      <Coverage entry={entry} />
      <Preview html={states} label={`${entry.name} states`} document />
      <h2>Attributes</h2>
      <BrowserTable
        label={`${entry.name} attributes`}
        columns={['Attribute', 'Values', 'Required']}
        rows={Object.entries(entry.attributes).map(([name, rule]) => [
          name,
          rule.values?.join(' · ') ?? rule.kind,
          rule.required ? 'Yes' : 'No',
        ])}
      />
      <h2>Slots</h2>
      <p>{entry.slots.length ? entry.slots.join(' · ') : 'Text label only; no named slots.'}</p>
      <h2>Tokens used</h2>
      <ul>
        {entry.tokens.map((token) => (
          <li key={token}>
            <code>{token}</code>
          </li>
        ))}
      </ul>
      <h2>Embed in Colab</h2>
      <p>Paste this markup into a Colab document. No script is needed.</p>
      <CopyCommand
        command={entry.examples[0]!.html}
        enabled
        copyLabel="Copy embed code"
        copiedText="Embed code copied."
        failedText="Copy failed. Select the markup to copy it."
      />
      <p>
        <a
          href={`https://github.com/tmt-ninja/tmt/blob/main/design/browser-ui/src/${entry.source}`}
        >
          View source
        </a>
      </p>
    </>
  );
}
const terminalInventory = [
  ['Dialog / Button', 'Forms, confirmation and actions', 'tmt-tui/components/dialog.rs; button.rs'],
  ['Modal / Outline', 'Framed overlays and outlines', 'tmt-tui/components/modal.rs; outline.rs'],
  [
    'Surface / Collection / List / Picker',
    'Markup, lists, tables and filtered choices',
    'tmt-tui/components/surface.rs; collection.rs; list.rs; picker.rs',
  ],
  [
    'ScrollState / StatusSlot',
    'Scroll ranges, status and busy state',
    'tmt-tui/components/scroll.rs; status.rs',
  ],
  [
    'KeyHelp / Editor / Completion',
    'Keyboard help, text entry and completion',
    'tmt-tui/components/key_help.rs; editor.rs; completion.rs',
  ],
  [
    'Strip / Terminal',
    'Styled text strips and terminal output',
    'tmt-tui/components/strip.rs; terminal.rs',
  ],
  ['Look / Picker surface / Menu surface', 'Role styles and choice surfaces', 'tmt-ops board'],
  ['Chip group / Buttons / Keycap', 'Option chips, actions and keyboard labels', 'tmt-ops board'],
  [
    'Tab strip / Top bar / Filter bar / Search field',
    'Navigation, filtering and search',
    'tmt-ops board',
  ],
  ['Input field', 'One-to-six-line text entry', 'tmt-ops board'],
  ['Section rule / Divider / Pane chrome', 'Separators and pane focus', 'tmt-ops board'],
  ['State cell / Activity', 'Status marks and text', 'tmt-ops board'],
  [
    'Tree row / Team strip / Header sort / Row chips',
    'Member trees, ordering and source chips',
    'tmt-ops board',
  ],
  [
    'Spark row / Meter / Stat tiles / Key value / Matrices',
    'Trends, gauges and summary data',
    'tmt-ops board',
  ],
  [
    'Timer rows / Empty state / Notice / Scroll',
    'Timers, empty states, notices and scrolling',
    'tmt-ops board',
  ],
  ['Markdown / Hyperlinks', 'Messages and clickable links', 'tmt-ops board'],
];
function GuiIndex() {
  const [category, setCategory] = useState('All');
  const categories = ['All', ...new Set(browserCatalog.map((entry) => entry.category))];
  return (
    <>
      <BrowserPageSubheader
        title="Components"
        count={browserCatalog.length}
        subtitle="Live widgets you can embed in Colab. Preview changes stay in this page."
      />
      <BrowserTabs
        label="Component categories"
        tabs={categories.map((value) => ({
          id: value,
          label: value,
          tabId: `design-category-${value}`,
          panelId: `design-category-panel-${value}`,
          count:
            value === 'All'
              ? browserCatalog.length
              : browserCatalog.filter((entry) => entry.category === value).length,
        }))}
        selected={category}
        onSelect={setCategory}
        variant="flat"
        presentation="compact"
        wrap
      >
        {(tab) =>
          tab.id !== category ? null : (
            <div className="design-grid">
              {browserCatalog
                .filter((entry) => category === 'All' || entry.category === category)
                .map((entry) => (
                  <article key={entry.tag} className={ui.card}>
                    <header>
                      <h2>
                        <Link
                          to="/design/$system/$component"
                          params={{ system: 'gui', component: entry.tag }}
                        >
                          {entry.name}
                        </Link>
                      </h2>
                      <p>{entry.category}</p>
                    </header>
                    <Preview html={entry.examples[0]!.html} label={`${entry.name} live preview`} />
                    <code>&lt;{entry.tag}&gt;</code>
                    <Coverage entry={entry} />
                  </article>
                ))}
            </div>
          )
        }
      </BrowserTabs>
      <h2>A script-free Colab document</h2>
      <Preview label="Embedded widgets in a Colab document" document html={designDocument} />
      <h2>Presentation inventory</h2>
      <p>
        Source exports, indexed documentation, gallery entries and discovered tests. An export
        without a widget tag remains a presentation API, not an author element.
      </p>
      <BrowserTable
        label="GUI presentation inventory"
        columns={['Component', 'Author element', 'Docs', 'Gallery', 'Test']}
        rows={browserPresentationInventory.map((entry) => [
          entry.component,
          entry.tag ?? '—',
          entry.docs ? '✓' : '—',
          entry.gallery ? '✓' : '—',
          entry.test ? '✓' : '—',
        ])}
      />
    </>
  );
}
export function DesignPage() {
  const params = useParams({ strict: false });
  const navigate = useNavigate();
  const system =
    designSystems.find((entry) => entry.id === params.system) ??
    (params.system ? undefined : designSystems[0]);
  const component = browserCatalog.find((entry) => entry.tag === params.component);
  if (!system || (params.component && (system.id !== 'gui' || !component)))
    return (
      <section className="design-page">
        <h2>Design entry not found</h2>
        <Link to="/design/$system" params={{ system: 'gui' }}>
          Open the GUI catalog
        </Link>
      </section>
    );
  return (
    <section className="design-page">
      <BrowserTabs
        label="Design systems"
        tabs={designSystems.map((entry) => ({
          id: entry.id,
          label: entry.name,
          tabId: `design-system-${entry.id}`,
          panelId: `design-system-panel-${entry.id}`,
        }))}
        selected={system.id}
        onSelect={(id) => {
          void navigate({ to: '/design/$system', params: { system: id } });
        }}
        variant="flush"
      >
        {(tab) =>
          tab.id !== system.id ? null : system.id === 'tui' ? (
            <>
              <BrowserPageSubheader
                title="Terminal components"
                count={terminalInventory.length}
                subtitle="Terminal widgets and styles from the real painters."
              />
              <BrowserTable
                label="TUI components"
                columns={['Component', 'Purpose', 'Owner']}
                rows={terminalInventory}
              />
              <p>
                Real terminal previews will use the shared painter catalog. They are not available
                in this build.
              </p>
              <p>
                <a href="https://github.com/tmt-ninja/tmt/issues/5784">Painter catalog</a> ·{' '}
                <a href="https://github.com/tmt-ninja/tmt/issues/5786">Terminal frame element</a>
              </p>
            </>
          ) : component ? (
            <>
              <BrowserPageSubheader
                title="States"
                count={component.examples.length}
                subtitle={component.purpose}
              />
              <Detail entry={component} />
            </>
          ) : (
            <GuiIndex />
          )
        }
      </BrowserTabs>
    </section>
  );
}
