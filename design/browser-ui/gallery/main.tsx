// Dev-only design gallery; see README "Gallery". Imports the leaf exactly as products do.
import '../generated/static.css';
import '../generated/ui.css';
import './gallery.css';
import favicon from '../generated/favicon.svg?url';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { sections } from './covered.js';
import { refreshForcedStates } from './force-states.js';
import { TokensSection } from './tokens.js';
import {
  ActionsSection,
  BrandSection,
  BusySection,
  ChipsSection,
  CommandSection,
  FieldsSection,
  HeaderSection,
  IconActionsSection,
  ListSection,
  MenusSection,
  NoticeSection,
  TabsSection,
  ToggleSection,
  CheckboxSection,
} from './components.js';
import { NotificationsSection, OverlaysSection, ShellSection, frameFor } from './frames.js';
import { UiLayerSection } from './ui-layer.js';
import { SpinnerCandidates } from './spinner-candidates.js';

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
        <TokensSection />
        <BrandSection />
        <ShellSection />
        <HeaderSection />
        <ActionsSection />
        <IconActionsSection />
        <BusySection />
        <SpinnerCandidates />
        <FieldsSection />
        <ToggleSection />
        <CheckboxSection />
        <ChipsSection />
        <MenusSection />
        <TabsSection />
        <ListSection />
        <CommandSection />
        <NoticeSection />
        <OverlaysSection />
        <NotificationsSection />
        <UiLayerSection />
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
