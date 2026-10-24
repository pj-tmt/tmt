import { useMedia } from './use-media.js';

export type PanelMode = 'column' | 'card' | 'sheet';

/** The viewport widths where a non-modal panel changes layout (concept 20); the leaf's guide names
 * the same two, and e2e/panel-mode.spec.ts holds the mode on both sides of each. */
export const PANEL_COLUMN_MIN_WIDTH = 1180;
export const PANEL_CARD_MIN_WIDTH = 761;

/** Where a non-modal panel sits at this width: a column beside the page, a card over its right edge,
 * or a sheet over its bottom. The host chooses; `BrowserPanel` does not infer a breakpoint. */
export function usePanelMode(): PanelMode {
  const column = useMedia(`(min-width: ${PANEL_COLUMN_MIN_WIDTH}px)`);
  const card = useMedia(`(min-width: ${PANEL_CARD_MIN_WIDTH}px)`);
  return column ? 'column' : card ? 'card' : 'sheet';
}
