import { defineConfig } from 'vite-plus';

export default defineConfig({
  test: {
    clearMocks: false,
    pool: 'threads',
    include: ['test/native/**/*.test.ts'],
    fileParallelism: false,
    testTimeout: 10_000,
    coverage: { enabled: false },
  },
});
