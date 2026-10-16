import { fileURLToPath } from "node:url";
import { blogBuild } from "./src/blog/build.mjs";
import { colabEmbedDev } from "./scripts/colab-embed-dev.ts";
import mdx from "@mdx-js/rollup";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import remarkFrontmatter from "remark-frontmatter";
import remarkGfm from "remark-gfm";
import remarkMdxFrontmatter from "remark-mdx-frontmatter";
import { defineConfig, lazyPlugins } from "vite-plus";
import { designTokens } from "../design/tokens/tokens-plugin.ts";

// GitHub Pages serves a project site under /<repo>/. SITE_BASE switches it,
// for example to "/" for a custom domain or a local preview.
const base = process.env.SITE_BASE ?? "/tmt/";

export default defineConfig(async ({ command }) => {
  const blog = base.startsWith("/")
    ? await blogBuild({
        root: fileURLToPath(new URL(".", import.meta.url)),
        base,
        origin: process.env.SITE_ORIGIN,
        development: command === "serve",
        demo: process.env.TMT_BLOG_DEMO === "1",
      })
    : undefined;
  return {
    base,
    build: blog
      ? {
          rolldownOptions: {
            input: [fileURLToPath(new URL("index.html", import.meta.url)), ...blog.entries],
          },
        }
      : undefined,
    server: {
      watch: process.env.VITE_COLAB_EMBED === "1" ? { usePolling: true, interval: 250 } : undefined,
    },
    plugins: lazyPlugins(() => [
      {
        enforce: "pre",
        // Translated pages (src/i18n) start with a YAML block: it is kept out of the
        // page and exported as `frontmatter`. See scripts/i18n-sync.mjs.
        ...mdx({
          remarkPlugins: [remarkGfm, remarkFrontmatter, remarkMdxFrontmatter],
          providerImportSource: "@mdx-js/react",
        }),
      },
      react({ include: /\.(mdx|tsx|ts)$/ }),
      designTokens(),
      ...(process.env.VITE_COLAB_EMBED === "1" ? [colabEmbedDev()] : []),
      tailwindcss(),
      ...(blog ? [blog.plugin] : []),
    ]),
  };
});
