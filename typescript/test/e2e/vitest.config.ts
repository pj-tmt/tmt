import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: { clearMocks: false,
    pool: 'threads',
    include: ['test/e2e/**/*.e2e.test.ts'],
    exclude: ['node_modules', 'dist'],
    fileParallelism: false,
    testTimeout: 15_000,
    hookTimeout: 5_000,
    maxConcurrency: 1,
    sequence: { concurrent: false, shuffle: false },
    reporters: ['default'],
    coverage: { enabled: false },
  },
});
