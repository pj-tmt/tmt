import { expect, test, type Page } from '@playwright/test';
import { pageAction } from '../test/page-actions.js';

/**
 * Typing in the Source editor of a page with many threads renders the editor and nothing else:
 * the page route, its thread panel and its other drawers are not rendered again for a keystroke.
 */
const fixture = '/test/ask-page-browser.tsx';
const run = (page: Page, method: string, argument?: unknown) =>
  page.evaluate(
    async ({ fixture, method, argument }) => (await import(fixture))[method](argument),
    { fixture, method, argument },
  );

/** Watched components, by the name a render is counted under. Drawers count by their panel. */
type Renders = Record<string, number>;

/**
 * Counts renders per component from React's DevTools commit hook, installed before React loads.
 * A function component rendered in a commit when its fiber is new or its props or hook list are
 * new objects: a render builds a fresh hook list, and a bailout keeps both.
 */
async function countRenders(page: Page) {
  await page.addInitScript(() => {
    const watched = new Set(['OpenPage', 'ThreadPanel', 'BrowserDrawer', 'SourceEditor']);
    const counts: Record<string, number> = {};
    const present: Record<string, number> = {};
    const seen = new WeakMap<object, { props: unknown; state: unknown }>();
    Object.defineProperty(window, 'componentRenders', { value: { counts, present } });
    interface Fiber {
      type?: { name?: string; displayName?: string } | string | null;
      memoizedProps?: { panel?: string } | null;
      memoizedState?: unknown;
      alternate?: Fiber | null;
      child?: Fiber | null;
      sibling?: Fiber | null;
    }
    const nameOf = (fiber: Fiber) => {
      const type = fiber.type;
      if (!type || typeof type === 'string') return undefined;
      const name = type.displayName ?? type.name;
      if (!name || !watched.has(name)) return undefined;
      return name === 'BrowserDrawer' ? `BrowserDrawer:${fiber.memoizedProps?.panel}` : name;
    };
    const visit = (root: Fiber | null | undefined) => {
      for (const key of Object.keys(present)) present[key] = 0;
      const stack: Fiber[] = root ? [root] : [];
      while (stack.length) {
        const fiber = stack.pop()!;
        const name = nameOf(fiber);
        if (name) {
          present[name] = (present[name] ?? 0) + 1;
          const last = seen.get(fiber) ?? (fiber.alternate ? seen.get(fiber.alternate) : undefined);
          if (!last || last.props !== fiber.memoizedProps || last.state !== fiber.memoizedState)
            counts[name] = (counts[name] ?? 0) + 1;
          const record = { props: fiber.memoizedProps, state: fiber.memoizedState };
          seen.set(fiber, record);
          if (fiber.alternate) seen.set(fiber.alternate, record);
        }
        if (fiber.sibling) stack.push(fiber.sibling);
        if (fiber.child) stack.push(fiber.child);
      }
    };
    Object.defineProperty(window, '__REACT_DEVTOOLS_GLOBAL_HOOK__', {
      value: {
        supportsFiber: true,
        renderers: new Map(),
        inject: () => 1,
        onCommitFiberRoot: (_id: number, root: { current: Fiber }) => visit(root.current),
        onCommitFiberUnmount: () => {},
        onPostCommitFiberRoot: () => {},
        checkDCE: () => {},
      },
    });
  });
}
const renders = (page: Page) =>
  page.evaluate(() => {
    const { counts, present } = (
      window as unknown as { componentRenders: { counts: Renders; present: Renders } }
    ).componentRenders;
    return { counts: { ...counts }, present: { ...present } };
  });
const frames = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
/** The counts once nothing has rendered for several frames in a row. */
async function quiet(page: Page) {
  let last = '';
  for (let steady = 0, tries = 0; steady < 5; tries++) {
    if (tries > 200) throw new Error('The page never went quiet');
    await frames(page);
    const now = JSON.stringify((await renders(page)).counts);
    steady = now === last ? steady + 1 : 0;
    last = now;
  }
  return JSON.parse(last) as Renders;
}

test('typing in the Source editor renders only the editor', async ({ page }) => {
  await countRenders(page);
  await page.goto('/');
  await run(page, 'mount', { statusAgents: true });
  await run(page, 'largePage', { paragraphs: 200, threads: 60 });
  await expect(
    page.frameLocator('#ask-page-fixture iframe').locator('[data-colab-thread]'),
  ).toHaveCount(60);
  await (await pageAction(page, 'Source')).click();
  const area = page.locator('#source-edit:visible');
  await expect(area).toBeVisible();
  await area.focus();
  await area.evaluate((node: HTMLTextAreaElement) =>
    node.setSelectionRange(node.value.length, node.value.length),
  );
  const before = await quiet(page);
  const typed = 'abcdefghij';
  await page.keyboard.type(typed);
  await expect(area).toHaveValue(new RegExp(`${typed}$`));
  const after = await quiet(page);
  const caused = Object.fromEntries(
    Object.keys({ ...before, ...after }).map((name) => [
      name,
      (after[name] ?? 0) - (before[name] ?? 0),
    ]),
  );
  const { present } = await renders(page);
  // The watched parts are on the page, so a zero below is a skipped render, not a missing part.
  expect(present).toMatchObject({ OpenPage: 1, ThreadPanel: 1, SourceEditor: 1 });
  expect(present['BrowserDrawer:agents']).toBe(1);
  expect(caused.SourceEditor).toBe(typed.length);
  const others = Object.fromEntries(
    Object.entries(caused).filter(
      ([name]) => name !== 'SourceEditor' && name !== 'BrowserDrawer:source',
    ),
  );
  expect(others).toEqual(Object.fromEntries(Object.keys(others).map((name) => [name, 0])));
  // The Source drawer holds the editor and its Save, so it may render with it, never more.
  expect(caused['BrowserDrawer:source'] ?? 0).toBeLessThanOrEqual(typed.length);
});
