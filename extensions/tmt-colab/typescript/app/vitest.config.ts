import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: { clearMocks: false, include: ['test/**/*.test.ts'], passWithNoTests: false },
});
