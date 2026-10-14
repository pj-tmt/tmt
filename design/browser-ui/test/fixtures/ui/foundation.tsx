import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Portal } from 'radix-ui';
import {
  BrowserHeader,
  BrowserAction,
  BrowserField,
  BrowserIconAction,
  BrowserMenu,
  BrowserChip,
  BrowserDrawer,
} from '../../../src/react.js';
import { UiTheme, useUiPortalContainer, type UiThemeMode } from '../../../src/ui/index.js';
import { PrimitiveFixtures } from './primitives.js';
import { LayerFixtures } from './layers.js';
import { SelectionFixtures } from './selection.js';

function Probe({ name }: { name: string }) {
  return (
    <div
      data-probe={name}
      tabIndex={0}
      className="tmt:bg-background tmt:text-foreground tmt:p-2 tmt:border-solid tmt:border-border tmt:border-(length:--tmt-foundation-rule-width) tmt:outline-ring tmt:focus:outline-solid tmt:focus:outline-(length:--tmt-foundation-focus-width) tmt:z-(--tmt-foundation-z-layer)"
    >
      Token probe {name}
    </div>
  );
}
function PortalProbe({ name }: { name: string }) {
  const container = useUiPortalContainer();
  return container ? (
    <Portal.Root container={container}>
      <Probe name={`${name}-portal`} />
    </Portal.Root>
  ) : null;
}
function Island({ name, theme }: { name: string; theme: UiThemeMode }) {
  return (
    <UiTheme theme={theme}>
      <Probe name={name} />
      <PortalProbe name={name} />
    </UiTheme>
  );
}
function Gallery() {
  const [theme, setTheme] = useState<UiThemeMode>('light');
  const [drawer, setDrawer] = useState(false);
  const [loaded, setLoaded] = useState(false);
  return (
    <>
      <section id="legacy">
        <BrowserHeader productLabel="Colab" title="Foundation coexistence" />
        <BrowserField
          controlId="legacy-input"
          label="Legacy field"
          renderControl={(props) => <input {...props} defaultValue="Kept" />}
        />
        <BrowserAction
          type="button"
          variant="primary"
          label="Legacy action"
          onActivate={() => {}}
        />
        <BrowserIconAction
          type="button"
          variant="text"
          label="Legacy icon"
          icon="+"
          onActivate={() => {}}
        />
        <BrowserMenu
          label="Legacy menu"
          items={[{ key: 'one', label: 'One' }]}
          onSelect={() => {}}
        />
        <BrowserChip label="Legacy chip" removeLabel="Remove legacy chip" onRemove={() => {}} />
        <BrowserAction
          type="button"
          variant="text"
          label="Legacy drawer"
          onActivate={() => setDrawer(true)}
        />
        <BrowserDrawer open={drawer} title="Legacy drawer" onDismiss={() => setDrawer(false)}>
          Retained legacy surface
        </BrowserDrawer>
      </section>
      <section aria-label="Infrastructure gallery">
        <h2>Slice1 infrastructure only; no shadcn components implemented</h2>
        <button
          onClick={() => {
            void import('@tmt/browser-ui/ui.css').then(() => setLoaded(true));
          }}
          disabled={loaded}
        >
          Load isolated UI CSS
        </button>
        <output data-css-loaded={loaded}>{loaded ? 'UI CSS loaded' : 'UI CSS not loaded'}</output>
        <select
          aria-label="Outer theme"
          value={theme}
          onChange={(event) => setTheme(event.target.value as UiThemeMode)}
        >
          <option>light</option>
          <option>dark</option>
          <option>system</option>
        </select>
        <UiTheme theme={theme}>
          <Probe name="outer" />
          <PortalProbe name="outer" />
          <Island name="nested-light" theme="light" />
          <Island name="nested-dark" theme="dark" />
          <Island name="nested-system" theme="system" />
          <PrimitiveFixtures />
          <LayerFixtures />
          <SelectionFixtures />
        </UiTheme>
        <div id="outside" className="tmt:bg-background tmt:p-2">
          Outside namespace
        </div>
        <input id="unscoped-input" aria-label="Unscoped control" defaultValue="No reset" />
      </section>
    </>
  );
}
export function mountFoundation() {
  const root = document.getElementById('root');
  if (!root) throw new Error('Missing private foundation fixture root');
  createRoot(root).render(<Gallery />);
}
