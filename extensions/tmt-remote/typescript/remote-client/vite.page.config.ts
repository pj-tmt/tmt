import { defineConfig } from 'vite-plus';
import sdk from './vite.config.js';
/** Remote product page; its public SDK import stays external and shares one channel. */
export default defineConfig({
  ...sdk,
  build: {
    ...sdk.build,
    lib: {
      entry: { 'settings-v1': 'src/settings-page.ts', 'invite-v1': 'src/invite-page.ts' },
      formats: ['es'],
      fileName: (_format, entryName) => `${entryName}.js`,
    },
    rollupOptions: {
      external: ['remote-browser-sdk'],
      output: { paths: { 'remote-browser-sdk': '/sdk/remote-v1.js' } },
    },
  },
});
