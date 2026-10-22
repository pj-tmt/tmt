// Dev-only design gallery; see README "Gallery". Imports the leaf exactly as products do.
import '../generated/static.css';
import '../generated/ui.css';
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

function Gallery() {
  return (
    <div className="gallery">
      <nav className="gallery-nav" aria-label="Sections">
        <strong>Design gallery</strong>
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
if (import.meta.hot) import.meta.hot.on('vite:afterUpdate', () => refreshForcedStates(document));
