import { useSyncExternalStore } from 'react';

/** The page-level preview settings (#5821): the viewer theme and the one radius knob, kept on the
 * document root and in localStorage, so every page and mock opens the way it was left. */
export type Theme = 'dark' | 'light';
export type Radius = '2' | '4' | 'current';
const root = document.documentElement;
const listeners = new Set<() => void>();
const stored = (key: string) => {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
};
const store = (key: string, value: string) => {
  try {
    localStorage.setItem(key, value);
  } catch {
    // A private window keeps the choice for this page only.
  }
};
const radiusStyle: Record<Radius, Record<string, string>> = {
  '2': { '--tmt-ui-ref-radius': '2px' },
  '4': { '--tmt-ui-ref-radius': '4px' },
  current: {
    '--tmt-ui-radius-s': '6px',
    '--tmt-ui-radius': '8px',
    '--tmt-ui-radius-l': '12px',
    '--tmt-ui-radius-badge': '999px',
  },
};
const allRadiusProperties = [
  ...new Set(Object.values(radiusStyle).flatMap((style) => Object.keys(style))),
];

let theme: Theme = stored('tmt-docs-theme') === 'light' ? 'light' : 'dark';
let radius: Radius = (['2', '4', 'current'] as const).find((r) => r === stored('tmt-docs-radius')) ?? '2';
function apply() {
  root.dataset.theme = theme;
  root.dataset.tmtPalette = 'b';
  for (const name of allRadiusProperties) root.style.removeProperty(name);
  for (const [name, value] of Object.entries(radiusStyle[radius])) root.style.setProperty(name, value);
  for (const listener of listeners) listener();
}
apply();

export function setTheme(next: Theme) {
  theme = next;
  store('tmt-docs-theme', next);
  apply();
}
export function setRadius(next: Radius) {
  radius = next;
  store('tmt-docs-radius', next);
  apply();
}
export const radiusLabel: Record<Radius, string> = { '2': '2 / 4', '4': '4 / 6', current: 'Current' };
export function usePreview(): { theme: Theme; radius: Radius } {
  const subscribe = (notify: () => void) => {
    listeners.add(notify);
    return () => listeners.delete(notify);
  };
  const snapshot = useSyncExternalStore(subscribe, () => `${theme}|${radius}`);
  const [t, r] = snapshot.split('|') as [Theme, Radius];
  return { theme: t, radius: r };
}
