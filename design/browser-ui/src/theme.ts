/**
 * The per-browser colour theme (#3937). The tokens answer to `data-theme` on the root: `light` and
 * `dark` pin a theme, and no attribute follows the device (`prefers-color-scheme`). The choice is
 * stored in this browser only; storage that is blocked or full never breaks the page, which then
 * follows the device.
 */
export const THEMES = ['system', 'light', 'dark'] as const;
export type ThemeChoice = (typeof THEMES)[number];

export const themeStorageKey = 'tmt-ui-theme';

/** The part of `Storage` the theme uses. */
export interface ThemeStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}
/** The part of the root element the theme sets. */
export interface ThemeRoot {
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
}

function browserStore(): ThemeStore | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

/** The stored choice; `system` when none is stored, storage is blocked or the value is unknown. */
export function storedTheme(store: ThemeStore | undefined = browserStore()): ThemeChoice {
  try {
    const value = store?.getItem(themeStorageKey);
    return THEMES.find((theme) => theme === value) ?? 'system';
  } catch {
    return 'system';
  }
}

/** Set the root's theme for a choice: `system` leaves the tokens to the device. */
export function applyTheme(choice: ThemeChoice, root: ThemeRoot = document.documentElement): void {
  if (choice === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', choice);
}

/** Apply the stored choice; call once when the app starts, before the first paint of a page. */
export function applyStoredTheme(root?: ThemeRoot, store?: ThemeStore): void {
  applyTheme(storedTheme(store), root);
}

/** Store a choice for this browser and apply it now. `system` clears the stored choice. */
export function setStoredTheme(
  choice: ThemeChoice,
  root?: ThemeRoot,
  store: ThemeStore | undefined = browserStore(),
): void {
  try {
    if (choice === 'system') store?.removeItem(themeStorageKey);
    else store?.setItem(themeStorageKey, choice);
  } catch {
    // The choice still applies to this page; it is just not remembered.
  }
  applyTheme(choice, root);
}
