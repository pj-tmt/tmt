import { expect, it } from 'vite-plus/test';
import { createShellTopBar, type ShellCommonControls } from '../src/topbar.js';

/** Validation happens before rendering or committing a root projection. */
function owner() {
  const root = { ownerDocument: { baseURI: 'https://door.test/' } } as HTMLElement;
  return createShellTopBar(root, {} as HTMLElement, {} as HTMLButtonElement);
}
const common = (liveSessions: number): ShellCommonControls => ({
  openSearch() {},
  remote: { kind: 'confirmed', liveSessions, href: '/settings' },
});

it.each([-1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1])(
  'refuses invalid live-session counts without installing controls (%s)',
  (count) => {
    const bar = owner();
    expect(() => bar.setCommonControls(common(count))).toThrow('non-negative integer');
    expect(() => bar.setTopBar()).toThrow('install common controls');
  },
);
it('permanently releases root bindings on disposal', () => {
  const bar = owner();
  bar.setCommonControls(common(0));
  bar.dispose();
  bar.dispose();
  bar.setCommonControls(common(6));
  expect(() => bar.setTopBar()).toThrow('disposed');
});

