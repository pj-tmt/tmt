/** How Colab frames a page (#3913). `column` and `full` get Colab's column inside the render frame;
 * `canvas` and `freeform` leave the author's own layout alone. A page without a layout is
 * `freeform`: every page is, until the layout is page metadata (#3926). */
export type PageLayout = 'column' | 'full' | 'canvas' | 'freeform';

/** Whether Colab owns the frame of a page. The widths and gaps live in `page-frame.css`, inside the
 * render frame, and in the parent's `.canvas[data-layout]` rules, which must agree. */
export const framedLayout = (layout: PageLayout | undefined): layout is 'column' | 'full' =>
  layout === 'column' || layout === 'full';

/** The column's width as the frame sets it on a block (`page-frame.css`): at most 1280 px, 16 px
 * from each side of a narrower window. A loading frame uses it too, so a page opens where it ends. */
export const columnInlineSize = 'min(1280px, 100% - 32px)';
