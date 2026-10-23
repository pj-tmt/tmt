import { expect, test } from '@playwright/test';
import {
  controlBoundaries,
  controlRadii,
  focusFrames,
  oneSidedBorders,
  textAtSurfaceEdge,
} from './audits';

// Deliberately broken, synthetic DOM proves each audit sees the rule it is meant to enforce.
// Every case repairs the same element so an empty result cannot pass by measuring nothing.
test('audit control: rounded one-sided border', async ({ page }) => {
  await page.setContent(
    '<main><div style="border-radius:8px;border-left:1px solid black">Box</div></main>',
  );
  expect(await oneSidedBorders(page, 'main')).toHaveLength(1);
  await page.locator('div').evaluate((el) => {
    el.style.border = '1px solid black';
  });
  expect(await oneSidedBorders(page, 'main')).toEqual([]);
});

test('audit control: text against a filled edge', async ({ page }) => {
  await page.setContent(
    '<main><div style="display:inline-block;background:black;color:white">Edge</div></main>',
  );
  expect(await textAtSurfaceEdge(page, 'main')).toHaveLength(1);
  await page.locator('div').evaluate((el) => {
    el.style.padding = '8px';
  });
  expect(await textAtSurfaceEdge(page, 'main')).toEqual([]);
});

test('audit control: inconsistent control radius', async ({ page }) => {
  await page.setContent(
    '<style>:root{--tmt-ui-radius:8px}</style><main><button class="tmt-ui-action" style="border-radius:4px">Action</button></main>',
  );
  expect((await controlRadii(page, 'main')).off).toHaveLength(1);
  await page.locator('button').evaluate((el) => {
    el.style.borderRadius = '8px';
  });
  expect((await controlRadii(page, 'main')).off).toEqual([]);
});

test('audit control: clipped or doubled focus frame', async ({ page }) => {
  await page.setContent(`<style>
    :root{--tmt-ui-color-focus:rgb(0,0,255);--tmt-ui-color-focus-destructive:rgb(255,0,0)}
    div{width:100px;height:40px;overflow:hidden;border-radius:20px;border:1px solid black}
    button{width:100px;height:40px;border:0;border-radius:0;background:white}
    button:focus-visible{outline:2px solid blue}
  </style><main><div><button>Focus</button></div></main>`);
  expect((await focusFrames(page, 'main')).clipped).toHaveLength(1);
  await page.addStyleTag({ content: 'div:focus-within{border-color:blue}' });
  const doubled = await focusFrames(page, 'main');
  expect(doubled.clipped).toEqual([]);
  expect(doubled.doubled).toHaveLength(1);
  await page.addStyleTag({ content: 'button:focus-visible{outline:none}' });
  const repaired = await focusFrames(page, 'main');
  expect(repaired.checked).toBe(1);
  expect(repaired.clipped).toEqual([]);
  expect(repaired.doubled).toEqual([]);
});

test('audit control: border-only control contrast', async ({ page }) => {
  await page.setContent(
    '<main style="background:white"><input type="checkbox" class="tmt-ui-checkbox" style="appearance:none;width:20px;height:20px;border:1px solid #ddd"></main>',
  );
  expect((await controlBoundaries(page, 'main')).borders.misses).toHaveLength(1);
  await page.locator('input').evaluate((el) => {
    el.style.borderColor = '#555';
  });
  const repaired = (await controlBoundaries(page, 'main')).borders;
  expect(repaired.measured).toBe(1);
  expect(repaired.misses).toEqual([]);
});

test('audit control: soft field needs distinct fill and visible label', async ({ page }) => {
  await page.setContent(
    '<main style="background:white"><input id="field" class="tmt-ui-field-control" style="background:white"></main>',
  );
  const broken = (await controlBoundaries(page, 'main')).fields;
  expect(broken.sameFill).toHaveLength(1);
  expect(broken.unlabelled).toHaveLength(1);
  await page.locator('input').evaluate((el) => {
    el.style.backgroundColor = '#ddd';
    const label = document.createElement('label');
    label.htmlFor = 'field';
    label.textContent = 'Field';
    el.before(label);
  });
  const repaired = (await controlBoundaries(page, 'main')).fields;
  expect(repaired.measured).toBe(1);
  expect(repaired.sameFill).toEqual([]);
  expect(repaired.unlabelled).toEqual([]);
});

test('audit control: a search field is named by its placeholder', async ({ page }) => {
  await page.setContent(
    '<main style="background:white"><input type="search" class="tmt-ui-field-control" style="background:#ddd"></main>',
  );
  expect((await controlBoundaries(page, 'main')).fields.unlabelled).toHaveLength(1);
  await page.locator('input').evaluate((el) => el.setAttribute('placeholder', 'Filter agents'));
  expect((await controlBoundaries(page, 'main')).fields.unlabelled).toEqual([]);
});

test('audit control: clipped glyphs are not painted on an outer surface edge', async ({ page }) => {
  await page.setContent(
    '<main style="box-sizing:border-box;width:120px;padding:10px;background:black;color:white"><div style="overflow:hidden"><span style="position:relative;white-space:nowrap">Clipped text</span></div></main>',
  );
  await page.locator('span').evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    // Put the raw range 1px inside the filled surface, beyond its transparent clip container.
    el.style.left = `${el.closest('main')!.getBoundingClientRect().right - 1 - range.getBoundingClientRect().right}px`;
  });
  const geometry = await page.locator('span').evaluate((el) => ({
    textRight: el.getBoundingClientRect().right,
    clipRight: el.parentElement!.getBoundingClientRect().right,
    surfaceRight: el.closest('main')!.getBoundingClientRect().right,
  }));
  expect(geometry.textRight).toBeGreaterThan(geometry.clipRight);
  expect(geometry.surfaceRight - geometry.textRight).toBeCloseTo(1);
  expect(await textAtSurfaceEdge(page, 'main')).toEqual([]);
  // The same range really does violate the rule when those glyphs become visible.
  await page.locator('div').evaluate((el) => {
    el.style.overflow = 'visible';
  });
  expect(await textAtSurfaceEdge(page, 'main')).toHaveLength(1);
});
