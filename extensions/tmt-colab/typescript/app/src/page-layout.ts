/** How Colab frames a page (#3913). `column` and `full` get Colab's column inside the render frame;
 * `canvas` and `freeform` leave the author's own layout alone. A page without `meta.layout` is
 * `freeform`; it is an absence, never a stored value (#3926). */
export type PageLayout = 'column' | 'full' | 'canvas' | 'freeform';

/** Whether Colab owns the frame of a page. The widths and gaps live in `page-frame.css`, inside the
 * render frame, and in the parent's `.canvas[data-layout]` rules, which must agree. */
export const framedLayout = (layout: PageLayout | undefined): layout is 'column' | 'full' =>
  layout === 'column' || layout === 'full';
