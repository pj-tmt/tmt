import { defineConfig } from 'vite-plus';

export default defineConfig({
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
