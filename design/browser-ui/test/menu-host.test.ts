import { expect, it } from 'vite-plus/test';
import { build } from 'vite-plus';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

it('bundles the native menu entry without a React, product, Node or other runtime dependency', async () => {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const production = path.join(root, 'src') + path.sep;
  let modules: string[] = [];
  let entryExports: string[] = [];
  await build({
    configFile: false, root, logLevel: 'silent',
    plugins: [{ name: 'inspect-menu-host-runtime', generateBundle(_options, bundle) {
      modules = [...this.getModuleIds()];
      for (const item of Object.values(bundle))
        if (item.type === 'chunk' && item.isEntry) entryExports = item.exports;
    } }],
    build: { write: false, minify: false, target: 'es2022',
      lib: { entry: path.join(root, 'src/menu-host.ts'), formats: ['es'] } },
  });
  expect(modules.length).toBeGreaterThan(5);
  expect(modules.filter((id) => !id.startsWith(production) && !id.startsWith('\0'))).toEqual([]);
  expect(entryExports).toEqual(['mountBrowserMenu']);
});
