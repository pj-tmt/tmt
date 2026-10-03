import { defineConfig } from 'vite-plus';

export default defineConfig({
  fmt: {
    semi: true,
    singleQuote: true,
    trailingComma: 'es5',
    printWidth: 100,
    tabWidth: 2,
    sortImports: false,
    sortPackageJson: false,
  },
  test: {
    clearMocks: false,
    pool: 'threads',
    include: ['test/tooling/**/*.test.ts'],
    exclude: ['test/e2e/**', 'node_modules', 'dist'],
    // Vitest 4 counts real, bounded synchronous subprocess and filesystem work.
    testTimeout: 10_000,
    // Limit simultaneous suites on high-core development machines.
    maxWorkers: 2,
  },
});
