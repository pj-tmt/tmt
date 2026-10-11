import { readFile, appendFile } from 'node:fs/promises';
import { defineConfig } from 'vite-plus';
/** Hosted-only asset: never embedded by Cargo or committed; its pin is reviewed. */
export default defineConfig({
  plugins: [
    {
      name: 'firebase-license-text',
      async closeBundle() {
        // Firebase's npm packages omit LICENSE; exact upstream firebase@13.0.0 text.
        // https://github.com/firebase/firebase-js-sdk/blob/firebase%4013.0.0/LICENSE
        const license = await readFile(
          new URL('./licenses/firebase-Apache-2.0.txt', import.meta.url),
          'utf8',
        );
        await appendFile(
          new URL('./dist/THIRD-PARTY-NOTICES.txt', import.meta.url),
          '\n## Firebase Apache-2.0 license text\n\n' + license,
        );
      },
    },
  ],
  build: {
    target: 'es2022',
    minify: true,
    sourcemap: false,
    outDir: 'dist',
    license: { fileName: 'THIRD-PARTY-NOTICES.txt' },
    lib: {
      entry: 'src/firebase-client.ts',
      formats: ['es'],
      fileName: () => 'remote-firebase-v1.js',
    },
  },
});
