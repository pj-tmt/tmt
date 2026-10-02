import { defineConfig } from 'vite';

/** One unminified ES module, embedded by the door as `/sdk/remote-v1.js`. */
export default defineConfig({
  build: {
    target: 'es2022',
    minify: false,
    emptyOutDir: false,
    outDir: '../../rust/tmt-remote/assets',
    lib: { entry: 'src/browser.ts', formats: ['es'], fileName: () => 'remote-v1.js' },
  },
});
