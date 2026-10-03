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
  test: {
    clearMocks: false,
    include: ['test/**/*.test.ts'],
    passWithNoTests: false,
  },
});
