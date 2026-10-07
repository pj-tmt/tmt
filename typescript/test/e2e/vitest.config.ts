import { RETIRED_E2E_FILES } from '../../scripts/e2e-shards.mjs';
import { defineConfig } from 'vite-plus';

export default defineConfig({
  test: {
    clearMocks: false,
    pool: 'threads',
    include: ['test/e2e/**/*.e2e.test.ts'],
    exclude: ['node_modules', 'dist', ...RETIRED_E2E_FILES.map((file) => `test/e2e/${file}`)],
    fileParallelism: false,
    testTimeout: 15_000,
    hookTimeout: 5_000,
    maxConcurrency: 1,
    sequence: { concurrent: false, shuffle: false },
    reporters: ['default'],
    coverage: { enabled: false },
  },
});
