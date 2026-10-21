/** Runs a change of what the page shows inside the browser's view transition when there is one and
 * the reader has not asked for less motion; otherwise it just runs. `change` resolves when the new
 * state is on screen, which is when the transition takes its second picture. Nothing else may rely
 * on the transition: the change happens either way. */
export async function withViewTransition(change: () => Promise<void> | void): Promise<void> {
  const start = (
    document as Document & { startViewTransition?: (run: () => Promise<void>) => unknown }
  ).startViewTransition;
  const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (!start || calm) {
    await change();
    return;
  }
  start.call(document, async () => {
    await change();
    // One frame lets the render that follows the change reach the page before the second picture.
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
  });
}
