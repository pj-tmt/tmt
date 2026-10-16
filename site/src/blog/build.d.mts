import type { Plugin } from "vite-plus";
export function blogBuild(options: {
  root: string;
  base: string;
  origin?: string;
  development: boolean;
  demo: boolean;
}): Promise<{ entries: string[]; plugin: Plugin }>;
