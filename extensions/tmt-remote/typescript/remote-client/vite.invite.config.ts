import { defineConfig } from 'vite-plus';
import sdk from './vite.config.js';

/** The standalone Hosting resource is self-contained and uses the same command catalog. */
export default defineConfig({
  ...sdk,
  build: {
    ...sdk.build,
    lib: { entry: 'src/invite-page.ts', formats: ['es'], fileName: () => 'invite-v1.js' },
  },
});
