import { afterEach, describe, expect, it, vi } from 'vite-plus/test';
import { visibleElement } from '../src/visible-element.js';

interface Box {
  top: number;
  bottom: number;
  overflow?: string;
}
/** A stand-in element: only the geometry and styles `visibleElement` reads. */
function element(box: Box, parent: HTMLElement | null = null) {
  return {
    isConnected: true,
    parentElement: parent,
    overflow: box.overflow ?? 'visible',
    getBoundingClientRect: () => ({
      left: 0,
      right: 100,
      top: box.top,
      bottom: box.bottom,
      width: 100,
      height: box.bottom - box.top,
    }),
  } as unknown as HTMLElement;
}
function viewport(height: number) {
  vi.stubGlobal('innerWidth', 390);
  vi.stubGlobal('innerHeight', height);
  vi.stubGlobal('getComputedStyle', (node: { overflow: string }) => ({
    visibility: 'visible',
    overflowX: node.overflow,
    overflowY: node.overflow,
  }));
}
afterEach(() => vi.unstubAllGlobals());

describe('visibleElement', () => {
  it('is not visible when the viewport and a scrolling ancestor do not overlap', () => {
    viewport(844);
    // The ancestor starts below the viewport, so the accumulated clip is empty even though the
    // child overlaps each side on its own.
    const scroller = element({ top: 900, bottom: 1100, overflow: 'auto' });
    expect(visibleElement(element({ top: 820, bottom: 940 }, scroller))).toBe(false);
  });

  it('is visible inside a scrolling ancestor that is on screen', () => {
    viewport(844);
    const scroller = element({ top: 100, bottom: 600, overflow: 'auto' });
    expect(visibleElement(element({ top: 500, bottom: 700 }, scroller))).toBe(true);
  });

  it('is not visible when scrolled out of an on-screen ancestor', () => {
    viewport(844);
    const scroller = element({ top: 100, bottom: 600, overflow: 'auto' });
    expect(visibleElement(element({ top: 650, bottom: 800 }, scroller))).toBe(false);
  });

  it('is visible on screen with no clipping ancestor', () => {
    viewport(844);
    expect(visibleElement(element({ top: 820, bottom: 940 }))).toBe(true);
  });
});
