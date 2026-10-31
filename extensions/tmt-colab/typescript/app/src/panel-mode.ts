import { useMedia } from './use-media.js';

export type PanelMode = 'card' | 'sheet';

/** Floating panels become sheets below this width. */
export const PANEL_CARD_MIN_WIDTH = 761;

/** The host chooses a floating card or sheet; BrowserPanel does not infer a breakpoint. */
export function usePanelMode(): PanelMode {
  return useMedia(`(min-width: ${PANEL_CARD_MIN_WIDTH}px)`) ? 'card' : 'sheet';
}
