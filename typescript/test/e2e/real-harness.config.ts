import { defineConfig } from 'vite-plus';
import e2eConfig from './vitest.config.js';

export default defineConfig({
  ...e2eConfig,
  cacheDir: '/tmp/real-harness-vite',
  test: {
    ...e2eConfig.test,
    include: ['test/e2e/real-harness-smoke.advisory.test.ts'],
    fileParallelism: false,
    maxConcurrency: 1,
    sequence: { concurrent: false, shuffle: false },
    testTimeout: 150_000,
    hookTimeout: 30_000,
    coverage: { enabled: false },
  },
});
