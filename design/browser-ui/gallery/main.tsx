// Dev-only design gallery; see README "Gallery". Imports the leaf exactly as products do.
import '../generated/static.css';
import '../generated/ui.css';
import '../generated/editorial.css';
import './gallery.css';
import './gallery-page.css';
import favicon from '../generated/favicon.svg?url';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { sections } from './covered.js';
import { refreshForcedStates } from './force-states.js';
import { GallerySections } from './sections.js';
import { frameFor } from './frames.js';

document.querySelector<HTMLLinkElement>('#favicon')!.href = favicon;
const params = new URLSearchParams(location.search);
const theme = params.get('theme');
if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
const frame = params.get('frame');
// Palette B (#5821): `?palette=b` scopes the whole gallery, every section and frame, in B; `radius`
// sets the one knob (2, 4) or restores today's values (current).
const palette = params.get('palette') === 'b' ? 'b' : undefined;
if (palette) document.documentElement.dataset.tmtPalette = palette;
const radius = params.get('radius');
const radiusStyle: Record<string, Record<string, string>> = {
  '2': { '--tmt-ui-ref-radius': '2px' },
  '4': { '--tmt-ui-ref-radius': '4px' },
  current: {
    '--tmt-ui-radius-s': '6px',
    '--tmt-ui-radius': '8px',
    '--tmt-ui-radius-l': '12px',
    '--tmt-ui-radius-badge': '999px',
  },
};
for (const [name, value] of Object.entries(radiusStyle[radius ?? ''] ?? {}))
  document.documentElement.style.setProperty(name, value);
/** The same page with one query value changed, keeping the section anchor. */
const withParam = (name: string, value: string | undefined) => {
  const next = new URLSearchParams(location.search);
  if (value === undefined) next.delete(name);
  else next.set(name, value);
  const query = next.toString();
  return `${query ? `?${query}` : location.pathname}${location.hash}`;
};

function PreviewSwitches() {
  return (
    <div className="gallery-preview" role="group" aria-label="Preview">
      <span>Palette</span>
      <a href={withParam('palette', undefined)} aria-current={palette ? undefined : 'true'}>
        Current
      </a>
      <a href={withParam('palette', 'b')} aria-current={palette ? 'true' : undefined}>
        B
      </a>
      <span>Radius</span>
      {(['2', '4', 'current'] as const).map((value) => (
        <a
          key={value}
          href={withParam('radius', value)}
          aria-current={(radius ?? '2') === value ? 'true' : undefined}
        >
          {value === 'current' ? 'Current' : `${value} / ${Number(value) + 2}`}
        </a>
      ))}
    </div>
  );
}

function Gallery() {
  return (
    <div className="gallery">
      <nav className="gallery-nav" aria-label="Sections">
        <strong>Design gallery</strong>
        <PreviewSwitches />
        {sections.map(([id, title]) => (
          <a key={id} href={`#${id}`}>
            {title}
          </a>
        ))}
      </nav>
      <main className="gallery-main">
        <GallerySections />
      </main>
    </div>
  );
}

if (frame) document.documentElement.dataset.frame = frame;
createRoot(document.querySelector('#root')!).render(
  <StrictMode>{frame ? frameFor(frame, params) : <Gallery />}</StrictMode>,
);

refreshForcedStates(document);
// The sections render after load, so the browser's own jump to `#section` finds nothing; redo it.
if (!frame && location.hash)
  requestAnimationFrame(() => document.getElementById(location.hash.slice(1))?.scrollIntoView());
if (import.meta.hot) import.meta.hot.on('vite:afterUpdate', () => refreshForcedStates(document));
