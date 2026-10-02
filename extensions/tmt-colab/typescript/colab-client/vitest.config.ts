import { defineConfig } from 'vite-plus';

export default defineConfig({
  test: {
    clearMocks: false,
    include: ['test/**/*.test.ts'],
    passWithNoTests: false,
  },
});
