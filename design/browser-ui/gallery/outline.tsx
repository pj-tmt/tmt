// The gallery's outline: its sections and the demos inside them, read from the rendered page.
import { BrowserMenu, type BrowserMenuItem } from '../src/menu.js';

export interface OutlineEntry {
  id: string;
  label: string;
  /** The section this entry belongs to; a section lists its own id. */
  section: string;
  depth: 0 | 1;
}

const slug = (text: string) =>
  text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');

/** Sections and their demos in page order. A demo gets an address from its title; a repeated
 * title in one section gets a counter, so every entry can be linked. */
export function readOutline(main: HTMLElement): OutlineEntry[] {
  const entries: OutlineEntry[] = [];
  for (const section of main.querySelectorAll<HTMLElement>('.gallery-section')) {
    const heading = section.querySelector(':scope > h2');
    entries.push({
      id: section.id,
      label: heading?.textContent ?? section.id,
      section: section.id,
      depth: 0,
    });
    const used = new Set<string>();
    for (const pair of section.querySelectorAll<HTMLElement>(':scope > .gallery-pair')) {
      const title = pair.querySelector(':scope > h3')?.textContent ?? '';
      if (!pair.id) {
        const base = `${section.id}-${slug(title) || 'demo'}`;
        let id = base;
        for (let count = 2; used.has(id); count++) id = `${base}-${count}`;
        pair.id = id;
      }
      used.add(pair.id);
      entries.push({ id: pair.id, label: title, section: section.id, depth: 1 });
    }
  }
  return entries;
}

/** Wide screens: a sticky list beside the page, the current entry highlighted. */
export function GalleryOutline({
  entries,
  current,
  onSelect,
}: {
  entries: readonly OutlineEntry[];
  current: string;
  onSelect(id: string): void;
}) {
  const currentSection = entries.find((entry) => entry.id === current)?.section;
  return (
    <nav className="gallery-outline" aria-label="On this page">
      <div className="tmt-ui-rail-sub-group">On this page</div>
      <ul className="tmt-ui-rail-sub">
        {entries.map((entry) => (
          <li key={entry.id}>
            <a
              className="tmt-ui-rail-sub-item"
              href={`#${entry.id}`}
              aria-current={entry.id === current ? 'location' : undefined}
              data-depth={entry.depth}
              data-open={entry.section === currentSection || undefined}
              onClick={(event) => {
                event.preventDefault();
                onSelect(entry.id);
              }}
            >
              <span className="tmt-ui-rail-label">{entry.label}</span>
            </a>
          </li>
        ))}
      </ul>
    </nav>
  );
}

/** Phones and narrow windows: the same outline as one dropdown in the header. */
export function OutlineMenu({
  entries,
  current,
  onSelect,
}: {
  entries: readonly OutlineEntry[];
  current: string;
  onSelect(id: string): void;
}) {
  const items: BrowserMenuItem[] = entries.map((entry) => ({
    key: entry.id,
    label: entry.label,
    role: 'menuitemradio',
    checked: entry.id === current,
    content: entry.depth ? (
      <span style={{ paddingInlineStart: 16 }}>{entry.label}</span>
    ) : undefined,
  }));
  return (
    <BrowserMenu
      label="On this page"
      triggerText="On this page"
      items={items}
      onSelect={onSelect}
    />
  );
}
