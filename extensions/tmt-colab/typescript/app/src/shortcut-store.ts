import {
  readShortcuts,
  shortcutNameError,
  shortcutSearch,
  type Shortcut,
  type ShortcutCollection,
} from './shortcuts.js';
import type { HomeSearch } from './page-list-filter.js';
import type { ProjectPins } from './bootstrap.js';
type StoragePort = Pick<Storage, 'getItem' | 'setItem'>;
/** A single browser-local record per space. Mutations reread it synchronously before writing. */
export class ShortcutStore {
  readonly key: string;
  constructor(
    space: string,
    private storage: StoragePort = {
      getItem: (key) => localStorage.getItem(key),
      setItem: (key, value) => localStorage.setItem(key, value),
    },
  ) {
    this.key = `tmt-colab-shortcuts-v1:${space}`;
  }
  read(): ShortcutCollection {
    return readShortcuts(this.storage.getItem(this.key));
  }
  #write(change: (value: ShortcutCollection) => void) {
    const value = this.read();
    change(value);
    readShortcuts(JSON.stringify(value));
    this.storage.setItem(this.key, JSON.stringify(value));
    if (typeof window !== 'undefined')
      window.dispatchEvent(new CustomEvent('colab-shortcuts', { detail: this.key }));
  }
  save(input: { id?: string; name: string; search: HomeSearch }): Shortcut {
    const name = input.name.trim();
    const error = shortcutNameError(name);
    if (error) throw new Error(error);
    const item: Shortcut = {
      id: input.id ?? crypto.randomUUID(),
      name,
      search: shortcutSearch(input.search),
    };
    this.#write((value) => {
      const index = value.items.findIndex((existing) => existing.id === item.id);
      if (input.id && index < 0) throw new Error('This shortcut was deleted.');
      if (index < 0) value.items.push(item);
      else value.items[index] = item;
    });
    return item;
  }
  delete(id: string) {
    this.#write((value) => {
      value.items = value.items.filter((item) => item.id !== id);
    });
  }
  move(id: string, direction: -1 | 1) {
    this.#write((value) => {
      const from = value.items.findIndex((item) => item.id === id),
        to = from + direction;
      if (from < 0 || to < 0 || to >= value.items.length) return;
      const [item] = value.items.splice(from, 1);
      value.items.splice(to, 0, item);
    });
  }
  reorder(id: string, before: string | null) {
    this.#write((value) => {
      const item = value.items.find((item) => item.id === id);
      if (!item || id === before) return;
      const target = before === null ? undefined : value.items.find((item) => item.id === before);
      if (before !== null && !target) return;
      value.items = value.items.filter((item) => item.id !== id);
      value.items.splice(target ? value.items.indexOf(target) : value.items.length, 0, item);
    });
  }
  migrate(pins: ProjectPins | undefined) {
    if (!pins || this.read().migrated) return;
    this.#write((value) => {
      if (value.migrated) return;
      if (pins.automatic === false)
        for (const name of pins.names)
          value.items.push({ id: crypto.randomUUID(), name, search: { project: [name] } });
      value.migrated = true;
    });
  }
}
