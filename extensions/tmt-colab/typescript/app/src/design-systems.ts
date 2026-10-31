import { browserElements } from '@tmt/browser-ui/elements-manifest';

/** Colab owns these routes; a future composed-shell entry only needs their link. */
export const designSystems = [
  { id: 'gui', name: 'GUI', description: 'Live browser-ui widgets, shared with the application.' },
  { id: 'tui', name: 'TUI', description: 'Terminal widgets and styles from the real painters.' },
] as const;
export function designRouteHeader(path: string): { title: string; crumb: string } {
  const [, , system, component] = path.split('/');
  const current = designSystems.find((entry) => entry.id === system);
  const widget = browserElements.find((entry) => entry.tag === component);
  const segments = ['Design', current?.name ?? 'GUI', widget?.name].filter(
    (value): value is string => !!value,
  );
  return { title: segments.at(-1)!, crumb: segments.slice(0, -1).join(' / ') };
}

/** Only catalog-owned paths enter legacy owner/hash navigation; unknown paths still refuse. */
export function isDesignRoute(path: string): boolean {
  if (path === '/design') return true;
  const parts = path.split('/');
  return (
    parts[1] === 'design' &&
    designSystems.some((entry) => entry.id === parts[2]) &&
    (parts.length === 3 ||
      (parts.length === 4 &&
        parts[2] === 'gui' &&
        browserElements.some((entry) => entry.tag === parts[3])))
  );
}
