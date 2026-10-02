import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv } from 'vite-plus';
import { officeDeployment } from './src/auth/firebase-config.js';

export default defineConfig(({ mode }) => {
  const deployment = officeDeployment(mode, loadEnv(mode, import.meta.dirname, 'VITE_'));
  const fileName = '.well-known/tmt-office.json';
  const source = deployment ? `${JSON.stringify(deployment)}\n` : undefined;
  return {
    fmt: {
      semi: true,
      singleQuote: true,
      trailingComma: 'es5',
      printWidth: 100,
      tabWidth: 2,
      ignorePatterns: ['dist/**', 'node_modules/**'],
      sortImports: false,
      sortPackageJson: false,
    },
    build: { license: { fileName: 'THIRD-PARTY-NOTICES.txt' } },
    plugins: [
      react(),
      {
        name: 'office-public-deployment',
        generateBundle() {
          if (source) this.emitFile({ type: 'asset', fileName, source });
        },
        configureServer(server) {
          server.middlewares.use((request, response, next) => {
            if (request.url !== `/${fileName}`) return next();
            response.setHeader('Cache-Control', 'no-store');
            response.setHeader('Content-Type', 'application/json');
            response.statusCode = source ? 200 : 404;
            response.end(source ?? '{"error":"DEPLOYMENT_UNAVAILABLE"}\n');
          });
        },
      },
    ],
    test: {
      clearMocks: false,
      environment: 'jsdom',
      include: ['src/**/*.test.{ts,tsx}'],
      setupFiles: ['./src/test-setup.ts'],
      passWithNoTests: false,
      restoreMocks: true,
    },
  };
});
