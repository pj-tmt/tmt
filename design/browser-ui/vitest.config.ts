import { defineConfig } from 'vite-plus';
import { lintConfig } from '../../typescript/scripts/lint-config.mjs';
export default defineConfig({
  lint: lintConfig,
  fmt: {
    // The checked distribution is minified by its generator; source CSS remains formatted.
    ignorePatterns: ['generated/static.css', 'generated/editorial.css'],
    singleQuote: true,
    trailingComma: 'all',
    printWidth: 100,
    sortImports: false,
    sortPackageJson: false,
  },
  test: { include: ['test/**/*.test.ts', 'test/**/*.test.tsx'], passWithNoTests: false },
});
