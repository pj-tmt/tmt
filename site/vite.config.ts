import mdx from "@mdx-js/rollup";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import remarkGfm from "remark-gfm";
import { defineConfig } from "vite";
import { designTokens } from "./src/design/tokens-plugin.ts";

// GitHub Pages serves a project site under /<repo>/. SITE_BASE switches it,
// for example to "/" for a custom domain or a local preview.
const base = process.env.SITE_BASE ?? "/tmt/";

export default defineConfig({
  base,
  plugins: [
    {
      enforce: "pre",
      ...mdx({ remarkPlugins: [remarkGfm], providerImportSource: "@mdx-js/react" }),
    },
    react({ include: /\.(mdx|tsx|ts)$/ }),
    designTokens(),
    tailwindcss(),
  ],
});
