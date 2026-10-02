import type { MDXContent } from "mdx/types";
import type { Page } from "../chapters";
import type { LangCode } from "./languages";

// A translated page is src/i18n/<lang>/<file>.mdx, named like the English
// chapter file it translates (start.mdx, drv-tmux.mdx). Its front matter is
// exported as `frontmatter`; `title` is the page title, and a missing title
// falls back to the English one. A page without a file falls back to the
// English original.
type Translation = { default: MDXContent; frontmatter?: { title?: unknown } };

const modules = import.meta.glob<Translation>("../i18n/*/*.mdx", { eager: true });

export type Localized = { title: string; Content: MDXContent; translated: boolean };

export function localize(lang: LangCode, page: Page): Localized {
  const module = lang === "en" ? undefined : modules[`../i18n/${lang}/${page.file}.mdx`];
  if (!module) return { title: page.title, Content: page.Content, translated: lang === "en" };
  const title = module.frontmatter?.title;
  return {
    title: typeof title === "string" && title ? title : page.title,
    Content: module.default,
    translated: true,
  };
}
