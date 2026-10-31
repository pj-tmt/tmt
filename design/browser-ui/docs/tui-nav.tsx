import type { ReactNode } from 'react';

/**
 * The docs' "TUI components" section and the /mock/tui route (#5835). gui-3 builds the pages in
 * docs/tui/ and the mock in docs/mock-tui/. Until they land, each link shows where its page comes
 * from. When they land, `tuiPages` and `TuiMock` here point at their entries.
 */
export const tuiPages: { slug: string; title: string; lead: string; render(): ReactNode }[] = [
  ['roles', 'Roles'],
  ['glyphs', 'Glyphs'],
  ['components', 'Components'],
  ['patterns', 'Patterns'],
].map(([slug, title]) => ({
  slug: slug!,
  title: title!,
  lead: 'The terminal design system, built in #5835.',
  render: () => <p className="docs-muted">This page lands with #5835.</p>,
}));

export function TuiMock({ path }: { path: string }) {
  return (
    <main className="docs-main">
      <p className="docs-muted">The TUI mock ({path}) lands with #5835.</p>
    </main>
  );
}
