import react from '@vitejs/plugin-react';
import { loadEnv } from 'vite';
import { defineConfig } from 'vitest/config';
import { officeDeployment } from './src/auth/firebase-config.js';

export default defineConfig(({ mode }) => {
  const deployment = officeDeployment(mode, loadEnv(mode, import.meta.dirname, 'VITE_'));
  const fileName = '.well-known/tmt-office.json';
  const source = deployment ? `${JSON.stringify(deployment)}\n` : undefined;
  return {
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
      environment: 'jsdom',
      include: ['src/**/*.test.{ts,tsx}'],
      setupFiles: ['./src/test-setup.ts'],
      passWithNoTests: false,
      restoreMocks: true,
    },
  };
});
