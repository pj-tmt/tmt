import { defineConfig } from 'vite-plus';

export default defineConfig({
  cacheDir: '/tmp/real-harness-vite',
  test: {
    include: ['test/e2e/real-harness-smoke.advisory.test.ts'],
    fileParallelism: false,
    maxConcurrency: 1,
    sequence: { concurrent: false, shuffle: false },
    testTimeout: 150_000,
    hookTimeout: 30_000,
    coverage: { enabled: false },
  },
});
