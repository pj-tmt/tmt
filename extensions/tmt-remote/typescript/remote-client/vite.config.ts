import { lintConfig } from '../../../../typescript/scripts/lint-config.mjs';
import { defineConfig } from 'vite-plus';

/** One unminified ES module, embedded by the door as `/sdk/remote-v1.js`. */
export default defineConfig({
  lint: lintConfig,
  fmt: {
    singleQuote: true,
    trailingComma: 'all',
    printWidth: 100,
    sortImports: false,
    sortPackageJson: false,
  },
  build: {
    target: 'es2022',
    minify: false,
    emptyOutDir: false,
    outDir: '../../rust/tmt-remote/assets',
    lib: { entry: 'src/browser.ts', formats: ['es'], fileName: () => 'remote-v1.js' },
  },
});
