import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    pool: 'threads',
    include: ['test/native/**/*.test.ts'],
    fileParallelism: false,
    testTimeout: 10_000,
    coverage: { enabled: false },
  },
});
