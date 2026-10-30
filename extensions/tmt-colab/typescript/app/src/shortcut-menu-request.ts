import { createContext } from 'react';

/** A Shortcuts command from the shell's module menu, held until the page list opens its dialog. */
export interface ShortcutMenuRequest {
  kind: 'save' | 'manage';
  id: number;
}
export const ShortcutMenuRequestSlot = createContext<{
  request: ShortcutMenuRequest | undefined;
  done(id: number): void;
} | null>(null);
