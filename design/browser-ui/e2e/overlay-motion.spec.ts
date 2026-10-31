import { expect, test, type Page } from '@playwright/test';

/** Overlays leave with the motion they arrived with: after a dismissal the surface stays mounted
 * with data-state=closed for the exit duration, then goes; under reduced motion it goes at once. */
type Case = {
  name: string;
  url: string;
  surface: string;
  dismiss: string;
  /** The exit duration: `base` for the Browser dialogs and drawers; under the ui set (#5821) `fast`
   * for a dialog and a sheet and `quick` (two thirds of fast) for menus, popovers and tooltips. */
  duration: 'base' | 'fast' | 'quick';
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
    duration: 'fast',
  },
  {
    name: 'UI drawer',
    url: '/?frame=ui-overlay&kind=sheet&theme=dark',
    surface: '[data-slot="sheet-content"]',
    dismiss: '[data-slot="sheet-content"] button >> text=Cancel',
    duration: 'fast',
  },
  {
    name: 'UI popover',
    url: '/?frame=ui-overlay&kind=popover&theme=dark',
    surface: '[data-slot="popover-content"]',
    dismiss: 'Escape',
    duration: 'quick',
  },
  {
    name: 'UI tooltip',
    url: '/?frame=ui-overlay&kind=tooltip&theme=dark',
    surface: '[data-slot="tooltip-content"]',
    dismiss: 'Escape',
    duration: 'quick',
  },
  {
    name: 'UI menu',
    url: '/?frame=ui-overlay&kind=menu&theme=dark',
    surface: '[data-slot="dropdown-menu-content"]',
    dismiss: 'Escape',
    duration: 'quick',
  },
];

async function durationOf(page: Page, token: 'base' | 'fast' | 'quick') {
  return page.evaluate((name) => {
    const probe = document.createElement('div');
    probe.style.transitionDuration =
      name === 'quick' ? 'calc(var(--tmt-ui-motion-fast) * 2 / 3)' : `var(--tmt-ui-motion-${name})`;
    document.body.append(probe);
    const seconds = parseFloat(getComputedStyle(probe).transitionDuration);
    probe.remove();
    return seconds * 1000;
  }, token);
}

type ExitProbe = {
  closed: boolean;
  animations: Animation[];
  initial: { playState: AnimationPlayState; playbackRate: number }[];
  observer: MutationObserver;
};

/** Capture and hold the real exit animation as soon as the host marks the surface closed.
 * Seeking its timeline proves the lifetime without treating a busy CI frame as extra motion. */
async function dismiss(page: Page, motion: Case, hold: boolean) {
  await page.evaluate(
    ({ selector, hold }) => {
      const observer = new MutationObserver(() => {
        const element = document.querySelector(selector);
        if (!probe.closed && element?.getAttribute('data-state') === 'closed') {
          probe.closed = true;
          probe.animations = element.getAnimations();
          probe.initial = probe.animations.map((animation) => ({
            playState: animation.playState,
            playbackRate: animation.playbackRate,
          }));
          if (hold)
            for (const animation of probe.animations) {
              animation.pause();
              animation.currentTime = 0;
            }
        }
      });
      const probe: ExitProbe = { closed: false, animations: [], initial: [], observer };
      (window as unknown as { __exitProbe: ExitProbe }).__exitProbe = probe;
      observer.observe(document, {
        subtree: true,
        childList: true,
        attributes: true,
        attributeFilter: ['data-state'],
      });
    },
    { selector: motion.surface, hold },
  );
  if (motion.dismiss === 'Escape') await page.keyboard.press('Escape');
  else await page.locator(motion.dismiss).click();
}

async function exitTimings(page: Page) {
  return page.evaluate(() =>
    (window as unknown as { __exitProbe: ExitProbe }).__exitProbe.animations.map((animation) => {
      const timing = animation.effect!.getComputedTiming();
      return {
        duration: timing.duration,
        delay: timing.delay,
        activeDuration: timing.activeDuration,
        endTime: timing.endTime,
      };
    }),
  );
}

async function releaseProbe(page: Page) {
  await page.evaluate(() => {
    (window as unknown as { __exitProbe: ExitProbe }).__exitProbe.observer.disconnect();
  });
}

for (const motion of cases) {
  test(`${motion.name} stays mounted as closed for its exit duration`, async ({ page }) => {
    await page.setViewportSize({ width: 1080, height: 620 });
    await page.goto(motion.url);
    await expect(page.locator(motion.surface).first()).toBeVisible();
    const duration = await durationOf(page, motion.duration);
    expect(duration).toBe({ base: 180, fast: 120, quick: 80 }[motion.duration]);
    await dismiss(page, motion, true);
    try {
      await expect
        .poll(() =>
          page.evaluate(() => (window as unknown as { __exitProbe: ExitProbe }).__exitProbe.closed),
        )
        .toBe(true);
      const timings = await exitTimings(page);
      const initial = await page.evaluate(
        () => (window as unknown as { __exitProbe: ExitProbe }).__exitProbe.initial,
      );
      for (const animation of initial)
        expect(animation).toEqual({ playState: 'running', playbackRate: 1 });
      // Native Drawer retains its closed, off-screen node even when there is no running motion.
      if (!motion.stays) expect(timings.length).toBeGreaterThan(0);
      for (const timing of timings)
        expect(timing).toEqual({ duration, delay: 0, activeDuration: duration, endTime: duration });
      await page.evaluate(async (duration) => {
        const probe = (window as unknown as { __exitProbe: ExitProbe }).__exitProbe;
        for (const animation of probe.animations) animation.currentTime = duration - 1;
        // Let layout and the host's effects settle while the paused timeline cannot finish.
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        );
      }, duration);
      await expect(page.locator(motion.surface)).toHaveAttribute('data-state', 'closed');
      await page.evaluate(async () => {
        const animations = (window as unknown as { __exitProbe: ExitProbe }).__exitProbe.animations;
        for (const animation of animations) animation.finish();
        await Promise.all(animations.map((animation) => animation.finished));
      });
      if (motion.stays)
        await expect(page.locator(motion.surface)).toHaveAttribute('data-state', 'closed');
      else await expect(page.locator(motion.surface)).toHaveCount(0);
    } finally {
      await releaseProbe(page);
    }
  });

  test(`${motion.name} leaves at once under reduced motion`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.setViewportSize({ width: 1080, height: 620 });
    await page.goto(motion.url);
    await expect(page.locator(motion.surface).first()).toBeVisible();
    expect(await durationOf(page, motion.duration)).toBe(0);
    await dismiss(page, motion, false);
    try {
      if (motion.stays) {
        await expect(page.locator(motion.surface)).toHaveAttribute('data-state', 'closed');
        expect(
          await page.locator(motion.surface).evaluate((node) => node.getAnimations().length),
        ).toBe(0);
      } else await expect(page.locator(motion.surface)).toHaveCount(0);
      // Radix can expose a zero-duration CSSAnimation for its animationend bookkeeping.
      // Reduced motion requires no effective exit time, rather than no Animation object.
      const timings = await exitTimings(page);
      for (const timing of timings)
        expect(timing).toEqual({ duration: 0, delay: 0, activeDuration: 0, endTime: 0 });
    } finally {
      await releaseProbe(page);
    }
  });
}
