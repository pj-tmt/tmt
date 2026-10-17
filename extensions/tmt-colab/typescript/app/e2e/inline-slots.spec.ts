import { expect, test } from '@playwright/test';
const fixture = '/test/inline-slots-browser.tsx';
for (const occupied of [249, 250])
  test(`reading slots preserve all ${occupied} existing controls within the shared cap`, async ({
    page,
  }) => {
    await page.goto('/');
    await page.evaluate(
      async ({ fixture, occupied }) => (await import(fixture)).mount({ occupied, readings: 2 }),
      { fixture, occupied },
    );
    const proof = () => page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture);
    await expect.poll(async () => (await proof())?.length).toBe(250);
    const slots = await proof();
    expect(slots.slice(0, occupied).map((slot: { id: string }) => slot.id)).toEqual(
      Array.from(
        { length: occupied },
        (_, index) => `10000000-0000-4000-8000-${String(index).padStart(12, '0')}`,
      ),
    );
    expect(slots.slice(occupied).map((slot: { id: string }) => slot.id)).toEqual(
      occupied === 249 ? ['10000000-0000-4000-8000-000000001000'] : [],
    );
  });

test('a reading ID collision keeps the original control and refuses only the reading slot', async ({
  page,
}) => {
  await page.goto('/');
  await page.evaluate(
    async (fixture) => (await import(fixture)).mount({ occupied: 1, readings: 1, collide: true }),
    fixture,
  );
  await expect
    .poll(async () => page.evaluate(async (fixture) => (await import(fixture)).proof(), fixture))
    .toEqual([{ id: '10000000-0000-4000-8000-000000000000', height: 2 }]);
  await expect(page.locator('[data-selected-readings]')).toHaveText('');
});
