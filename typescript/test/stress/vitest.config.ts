import { defineConfig } from 'vite-plus';

export default defineConfig({
  test: {
    clearMocks: false,
    pool: 'threads',
    include: ['test/stress/**/*.test.ts'],
    fileParallelism: false,
    testTimeout: 10_000,
    coverage: { enabled: false },
  },
});
