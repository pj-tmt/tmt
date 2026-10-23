import { expect, test, type Page } from '@playwright/test';

/** Overlays leave with the motion they arrived with: after a dismissal the surface stays mounted
 * with data-state=closed for the exit duration, then goes; under reduced motion it goes at once. */
type Case = {
  name: string;
  url: string;
  surface: string;
  dismiss: string;
  /** The exit duration token: `base` for dialogs and drawers, `fast` for menus and popovers. */
  duration: 'base' | 'fast';
  /** A Browser drawer stays in the document (closed, off-screen); the others unmount. */
  stays?: boolean;
};

const cases: Case[] = [
  {
    name: 'Browser dialog',
    url: '/?frame=dialog&theme=dark',
    surface: 'dialog.tmt-ui-dialog',
    dismiss: '.tmt-ui-dialog-actions .tmt-ui-action >> nth=0',
    duration: 'base',
  },
  {
    name: 'Browser drawer',
    url: '/?frame=drawer&theme=dark',
    surface: 'dialog.tmt-ui-drawer',
    dismiss: '.tmt-ui-drawer-actions .tmt-ui-action >> nth=0',
    duration: 'base',
    stays: true,
  },
  {
    name: 'Browser notification',
    url: '/?frame=notifications&theme=dark',
    surface: '.tmt-ui-notification[data-notification="n1"]',
    dismiss: '[data-notification="n1"] .tmt-ui-notification-heading button',
    duration: 'base',
  },
  {
    name: 'UI dialog',
    url: '/?frame=ui-overlay&kind=dialog&theme=dark',
    surface: '[data-slot="dialog-content"]',
    dismiss: '[data-slot="dialog-content"] button >> text=Cancel',
    duration: 'base',
  },
  {
    name: 'UI drawer',
    url: '/?frame=ui-overlay&kind=sheet&theme=dark',
    surface: '[data-slot="sheet-content"]',
    dismiss: '[data-slot="sheet-content"] button >> text=Cancel',
    duration: 'base',
  },
  {
    name: 'UI popover',
    url: '/?frame=ui-overlay&kind=popover&theme=dark',
    surface: '[data-slot="popover-content"]',
    dismiss: 'Escape',
    duration: 'fast',
  },
  {
    name: 'UI tooltip',
    url: '/?frame=ui-overlay&kind=tooltip&theme=dark',
    surface: '[data-slot="tooltip-content"]',
    dismiss: 'Escape',
    duration: 'fast',
  },
  {
    name: 'UI menu',
    url: '/?frame=ui-overlay&kind=menu&theme=dark',
    surface: '[data-slot="dropdown-menu-content"]',
    dismiss: 'Escape',
    duration: 'fast',
  },
];

async function durationOf(page: Page, token: 'base' | 'fast') {
  return page.evaluate((name) => {
    const probe = document.createElement('div');
    probe.style.transitionDuration = `var(--tmt-ui-motion-${name})`;
    document.body.append(probe);
    const seconds = parseFloat(getComputedStyle(probe).transitionDuration);
    probe.remove();
    return seconds * 1000;
  }, token);
}

/** Dismisses the surface and reports when it was marked closed and when it left the document
 * (null while it stays), timed by the page's own observers rather than by sampling. */
async function leave(page: Page, test: Case) {
  await page.evaluate((selector) => {
    const log = { closedAt: null as number | null, goneAt: null as number | null };
    (window as unknown as { __leave: typeof log }).__leave = log;
    const check = () => {
      const element = document.querySelector(selector);
      if (element?.getAttribute('data-state') === 'closed' && log.closedAt === null)
        log.closedAt = performance.now();
      if (!element && log.closedAt !== null && log.goneAt === null) log.goneAt = performance.now();
    };
    new MutationObserver(check).observe(document, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: ['data-state'],
    });
  }, test.surface);
  if (test.dismiss === 'Escape') await page.keyboard.press('Escape');
  else await page.locator(test.dismiss).click();
  if (test.stays) {
    await expect(page.locator(test.surface)).toHaveAttribute('data-state', 'closed');
    await page.locator(test.surface).evaluate(async (element) => {
      await Promise.all(
        element.getAnimations().map((animation) => animation.finished.catch(() => {})),
      );
    });
  } else {
    await expect(page.locator(test.surface)).toHaveCount(0);
  }
  return page.evaluate(
    () =>
      (window as unknown as { __leave: { closedAt: number | null; goneAt: number | null } })
        .__leave,
  );
}

for (const motion of cases) {
  test(`${motion.name} stays mounted as closed for its exit duration`, async ({ page }) => {
    await page.setViewportSize({ width: 1080, height: 620 });
    await page.goto(motion.url);
    await expect(page.locator(motion.surface).first()).toBeVisible();
    const duration = await durationOf(page, motion.duration);
    expect(duration).toBe(motion.duration === 'base' ? 180 : 120);
    const { closedAt, goneAt } = await leave(page, motion);
    expect(closedAt, 'marked data-state=closed').not.toBeNull();
    if (motion.stays) {
      // The drawer stays in the document, closed and off-screen, after the slide.
      expect(goneAt).toBeNull();
      await expect(page.locator(motion.surface)).toHaveAttribute('data-state', 'closed');
    } else {
      expect(goneAt, 'left the document').not.toBeNull();
      // Mounted as closed for the exit duration (a frame of slack either way).
      expect(goneAt! - closedAt!).toBeGreaterThan(duration * 0.8);
      expect(goneAt! - closedAt!).toBeLessThan(duration * 2);
    }
  });

  test(`${motion.name} leaves at once under reduced motion`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setViewportSize({ width: 1080, height: 620 });
    await page.goto(motion.url);
    await expect(page.locator(motion.surface).first()).toBeVisible();
    expect(await durationOf(page, motion.duration)).toBe(0);
    const { closedAt, goneAt } = await leave(page, motion);
    if (motion.stays) {
      expect(await page.evaluate(() => document.getAnimations().length)).toBe(0);
    } else if (closedAt !== null) {
      expect(goneAt! - closedAt).toBeLessThan(60);
    }
  });
}
