import { defineConfig } from 'vitest/config';
export default defineConfig({
  base: './',
  build: {
    target: 'chrome137',
    rolldownOptions: {
      input: { popup: 'popup.html', background: 'src/background.ts' },
      output: { entryFileNames: '[name].js' },
    },
  },
  test: { clearMocks: false, include: ['src/**/*.test.ts'], passWithNoTests: false },
});
