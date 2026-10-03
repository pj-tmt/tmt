import { lintConfig } from '../../../../../../typescript/scripts/lint-config.mjs';
import { defineConfig } from 'vite-plus';

export default defineConfig({
  lint: lintConfig,
  fmt: {
    semi: true,
    singleQuote: true,
    trailingComma: 'es5',
    printWidth: 100,
    tabWidth: 2,
    ignorePatterns: ['dist/**', 'node_modules/**'],
    sortImports: false,
    sortPackageJson: false,
  },
  test: {
    clearMocks: false,
    include: ['test/**/*.test.ts'],
    passWithNoTests: false,
    restoreMocks: true,
  },
});
