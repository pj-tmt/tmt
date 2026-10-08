import { defineConfig } from 'vite-plus';

export default defineConfig({
  resolve: {
    alias: { 'remote-browser-sdk': new URL('./src/browser.ts', import.meta.url).pathname },
  },
  test: { clearMocks: false, include: ['test/**/*.test.ts'], passWithNoTests: false },
});
