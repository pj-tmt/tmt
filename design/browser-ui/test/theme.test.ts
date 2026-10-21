import { expect, it } from 'vite-plus/test';
import {
  applyStoredTheme,
  setStoredTheme,
  storedTheme,
  themeStorageKey,
  type ThemeRoot,
  type ThemeStore,
} from '../src/theme.js';

function root() {
  const attributes = new Map<string, string>();
  const node: ThemeRoot = {
    setAttribute: (name, value) => void attributes.set(name, value),
    removeAttribute: (name) => void attributes.delete(name),
  };
  return { node, theme: () => attributes.get('data-theme') };
}
function store(initial?: string) {
  const items = new Map<string, string>(initial === undefined ? [] : [[themeStorageKey, initial]]);
  const node: ThemeStore = {
    getItem: (key) => items.get(key) ?? null,
    setItem: (key, value) => void items.set(key, value),
    removeItem: (key) => void items.delete(key),
  };
  return { node, items };
}
const blocked: ThemeStore = {
  getItem() {
    throw new Error('blocked');
  },
  setItem() {
    throw new Error('blocked');
  },
  removeItem() {
    throw new Error('blocked');
  },
};

it('follows the device when nothing, or something unknown, is stored', () => {
  expect(storedTheme(store().node)).toBe('system');
  expect(storedTheme(store('sepia').node)).toBe('system');
  expect(storedTheme(undefined)).toBe('system');
  const page = root();
  applyStoredTheme(page.node, store().node);
  expect(page.theme()).toBeUndefined();
});

it('applies a stored light or dark choice to the root', () => {
  for (const choice of ['light', 'dark'] as const) {
    const page = root();
    applyStoredTheme(page.node, store(choice).node);
    expect(page.theme()).toBe(choice);
  }
});

it('stores a pinned choice, and system clears it and the root attribute', () => {
  const page = root();
  const kept = store();
  setStoredTheme('dark', page.node, kept.node);
  expect(kept.items.get(themeStorageKey)).toBe('dark');
  expect(page.theme()).toBe('dark');
  setStoredTheme('system', page.node, kept.node);
  expect(kept.items.has(themeStorageKey)).toBe(false);
  expect(page.theme()).toBeUndefined();
});

it('still applies the choice to this page when storage is blocked', () => {
  const page = root();
  expect(storedTheme(blocked)).toBe('system');
  setStoredTheme('light', page.node, blocked);
  expect(page.theme()).toBe('light');
});
