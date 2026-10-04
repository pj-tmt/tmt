import { lintConfig } from '../../../../typescript/scripts/lint-config.mjs';
import { defineConfig } from 'vite-plus';
import { readFileSync } from 'node:fs';
import react from '@vitejs/plugin-react';
import { designTokens } from '../../../../design/tokens/tokens-plugin.ts';

export default defineConfig(({ mode }) => ({
  lint: lintConfig,
  fmt: {
    singleQuote: true,
    trailingComma: 'all',
    printWidth: 100,
    sortImports: false,
    sortPackageJson: false,
  },
  base: './',
  plugins: [
    react(),
    designTokens(),
    {
      name: 'colab-renderer-policy',
      configureServer(server) {
        // Use the native policy owner, avoiding a second dev-only policy copy.
        const source = readFileSync(
          new URL('../../rust/tmt-colab/src/assets.rs', import.meta.url),
          'utf8',
        );
        const policy = source.match(/pub const RENDERER_POLICY: &str = "([^"]+)";/)?.[1];
        if (!policy) throw new Error('Missing native renderer policy');
        server.middlewares.use((request, response, next) => {
          const path = request.url?.split('?')[0] ?? '';
          if (
            path === '/assets/recovery.js' ||
            /^\/r\/[a-z0-9]+\/x\/colab\/assets\/recovery\.js$/.test(path)
          ) {
            request.url = '/src/guidance.ts';
          }
          if (
            path === '/renderer.html' ||
            /^\/r\/[a-z0-9]+\/x\/colab\/renderer\.html$/.test(path)
          ) {
            // Mirror the native exact renderer route for mounted protocol fixtures.
            // Vite otherwise serves the app fallback at this prefix.
            request.url = '/renderer.html';
            response.setHeader('Content-Security-Policy', policy);
            response.setHeader('Referrer-Policy', 'no-referrer');
            response.setHeader('X-Content-Type-Options', 'nosniff');
          }
          next();
        });
      },
    },
  ],
  // `reader` is the public read-only entry: one fixed-name script, stylesheet and decoder worker,
  // so the native server can allowlist exactly these files for unpaired browsers.
  define: mode === 'reader' ? { 'process.env.NODE_ENV': '"production"' } : undefined,
  worker:
    mode === 'reader'
      ? { format: 'es', rollupOptions: { output: { entryFileNames: 'assets/reader-fold.js' } } }
      : undefined,
  build:
    mode === 'recovery'
      ? {
          emptyOutDir: false,
          lib: {
            entry: new URL('./src/guidance.ts', import.meta.url).pathname,
            formats: ['es'],
            fileName: () => 'assets/recovery.js',
          },
        }
      : mode === 'reader'
        ? {
            emptyOutDir: false,
            copyPublicDir: false,
            lib: {
              entry: new URL('./src/reader-main.tsx', import.meta.url).pathname,
              formats: ['es'],
              fileName: () => 'assets/reader.js',
              cssFileName: 'assets/reader',
            },
          }
        : { license: { fileName: 'THIRD-PARTY-NOTICES.txt' } },
}));
