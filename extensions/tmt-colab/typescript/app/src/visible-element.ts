/** Observer delivery may trail a scroll. Recheck actual clipped geometry at read admission. */
export function visibleElement(node: HTMLElement | null): boolean {
  if (!node?.isConnected || getComputedStyle(node).visibility !== 'visible') return false;
  const rect = node.getBoundingClientRect();
  if (!rect.width || !rect.height) return false;
  let left = 0,
    top = 0,
    right = innerWidth,
    bottom = innerHeight;
  for (let parent = node.parentElement; parent; parent = parent.parentElement) {
    const style = getComputedStyle(parent),
      box = parent.getBoundingClientRect();
    if (style.overflowX !== 'visible') {
      left = Math.max(left, box.left);
      right = Math.min(right, box.right);
    }
    if (style.overflowY !== 'visible') {
      top = Math.max(top, box.top);
      bottom = Math.min(bottom, box.bottom);
    }
  }
  // Ancestors that do not overlap each other leave no clip for the node to show through.
  if (left >= right || top >= bottom) return false;
  return rect.right > left && rect.left < right && rect.bottom > top && rect.top < bottom;
}
