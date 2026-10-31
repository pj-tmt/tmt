import { expect, test } from 'vite-plus/test';
import { designSystems, designRouteTitle, isDesignRoute } from '../src/design-systems.js';

test('catalog addresses do not depend on a rail or menu registration', () => {
  expect(designSystems.map((entry) => entry.id)).toEqual(['gui', 'tui']);
  for (const path of ['/design', '/design/gui', '/design/tui', '/design/gui/tmt-ui-button'])
    expect(isDesignRoute(path)).toBe(true);
  for (const path of [
    '/design/unknown',
    '/design/tui/tmt-ui-button',
    '/design/gui/unknown',
    '/design/../settings',
    '/design/gui/tmt-ui-button/extra',
  ])
    expect(isDesignRoute(path)).toBe(false);
  expect(designRouteTitle('/design/gui/tmt-ui-button')).toBe('Design / GUI / Action');
});
