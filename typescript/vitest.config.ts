import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    pool: 'threads',
    include: ['test/tooling/**/*.test.ts'],
    exclude: ['test/e2e/**', 'node_modules', 'dist'],
    testTimeout: 1000,
    // Tooling cases spawn real bounded subprocesses. Limit simultaneous suites
    // instead of inflating their timeouts on high-core development machines.
    maxWorkers: 2,
  },
});
