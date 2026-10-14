import { defineConfig } from 'vite-plus';
/** Private acceptance bundle; never copied into a served or Colab release. */
export default defineConfig({
  build: {
    target: 'es2022',
    sourcemap: false,
    outDir: 'dist/emulator',
    lib: {
      entry: 'test/emulator/browser-entry.ts',
      formats: ['es'],
      fileName: () => 'browser-entry.js',
    },
  },
});
