import { defineConfig } from 'vite-plus';
import react from '@vitejs/plugin-react';
import { designTokens } from '../../../../design/tokens/tokens-plugin.ts';

export default defineConfig({
  fmt: {
    singleQuote: true,
    trailingComma: 'all',
    printWidth: 100,
    sortImports: false,
    sortPackageJson: false,
  },
  plugins: [react(), designTokens()],
  build: { license: { fileName: 'THIRD-PARTY-NOTICES.txt' } },
});
