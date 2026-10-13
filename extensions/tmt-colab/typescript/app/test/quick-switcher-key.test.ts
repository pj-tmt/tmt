import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import {
  admitsSwitcherShortcut,
  switcherModifiers,
  usesCommandKey,
} from '../src/quick-switcher-key.js';

afterEach(() => vi.unstubAllGlobals());

function key(changes: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return {
    key: 'k',
    ctrlKey: false,
    metaKey: true,
    altKey: false,
    shiftKey: false,
    isTrusted: true,
    defaultPrevented: false,
    repeat: false,
    isComposing: false,
    keyCode: 0,
    composedPath: () => [],
    ...changes,
  } as KeyboardEvent;
}

describe('one admitted quick-switcher shortcut', () => {
  it('uses only the platform modifier, independent of letter case', () => {
    vi.stubGlobal('navigator', { platform: 'MacIntel' });
    expect(admitsSwitcherShortcut(key())).toBe(true);
    expect(admitsSwitcherShortcut(key({ key: 'K' }))).toBe(true);
    expect(admitsSwitcherShortcut(key({ ctrlKey: true, metaKey: false }))).toBe(false);
    vi.stubGlobal('navigator', { platform: 'Linux x86_64' });
    expect(admitsSwitcherShortcut(key({ ctrlKey: true, metaKey: false }))).toBe(true);
    expect(admitsSwitcherShortcut(key())).toBe(false);
    expect(usesCommandKey('iPad')).toBe(true);
    expect(usesCommandKey('Win32')).toBe(false);
  });
  it('refuses synthetic, consumed, repeated, composing and modified keys with a valid positive control', () => {
    vi.stubGlobal('navigator', { platform: 'MacIntel' });
    expect(admitsSwitcherShortcut(key())).toBe(true);
    for (const change of [
      { isTrusted: false },
      { defaultPrevented: true },
      { repeat: true },
      { isComposing: true },
      { keyCode: 229 },
      { altKey: true },
      { shiftKey: true },
      { ctrlKey: true },
      { metaKey: false },
      { key: 'c' },
    ])
      expect(admitsSwitcherShortcut(key(change))).toBe(false);
  });
  it('keeps editor shortcuts, including a composed path through an editable ancestor', () => {
    vi.stubGlobal('navigator', { platform: 'MacIntel' });
    class Element extends EventTarget {
      isContentEditable = false;
      constructor(private editor: boolean) {
        super();
      }
      closest() {
        return this.editor ? this : null;
      }
    }
    vi.stubGlobal('HTMLElement', Element);
    const plain = new Element(false);
    const editor = new Element(true);
    expect(admitsSwitcherShortcut(key({ composedPath: () => [plain] }))).toBe(true);
    expect(admitsSwitcherShortcut(key({ composedPath: () => [plain, editor] }))).toBe(false);
    plain.isContentEditable = true;
    expect(admitsSwitcherShortcut(key({ composedPath: () => [plain] }))).toBe(false);
  });
  it('applies the same exact modifier rule to an untrusted cosmetic request without granting a gesture', () => {
    for (const command of [false, true]) {
      const valid = { ctrlKey: !command, metaKey: command, altKey: false, shiftKey: false };
      expect(switcherModifiers(valid, command)).toBe(true);
      for (const change of [
        { ctrlKey: false, metaKey: false },
        { ctrlKey: true, metaKey: true },
        { altKey: true },
        { shiftKey: true },
      ])
        expect(switcherModifiers({ ...valid, ...change }, command)).toBe(false);
    }
  });
});
