import { defineConfig } from 'vitest/config';

export default defineConfig({
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
