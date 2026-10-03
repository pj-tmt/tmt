import { lintConfig } from '../../../../typescript/scripts/lint-config.mjs';
import { defineConfig } from 'vite-plus';
export default defineConfig({
  lint: lintConfig,
  fmt: {
    singleQuote: true,
    trailingComma: 'all',
    printWidth: 100,
    sortImports: false,
    sortPackageJson: false,
  },
  base: './',
  build: {
    target: 'chrome137',
    rolldownOptions: {
      input: { popup: 'popup.html', background: 'src/background.ts' },
      output: { entryFileNames: '[name].js' },
    },
  },
  test: { clearMocks: false, include: ['src/**/*.test.ts'], passWithNoTests: false },
});
