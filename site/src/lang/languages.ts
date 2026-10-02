export type LangCode = "en" | "ja" | "zh";

export type Language = {
  code: LangCode;
  // The URL prefix: English lives at the root, the others under /ja and /zh.
  prefix: string;
  label: string;
  // The value for <html lang> and hreflang; Traditional Chinese is zh-Hant.
  htmlLang: string;
};

export const languages: Language[] = [
  { code: "en", prefix: "", label: "EN", htmlLang: "en" },
  { code: "ja", prefix: "/ja", label: "日本語", htmlLang: "ja" },
  { code: "zh", prefix: "/zh", label: "中文", htmlLang: "zh-Hant" },
];

export function languageOf(code: LangCode): Language {
  return languages.find((language) => language.code === code) ?? languages[0];
}

// Splits a router pathname into its language and the page path that language shares.
export function splitLang(pathname: string): { lang: LangCode; path: string } {
  const clean = pathname.replace(/\/+$/, "") || "/";
  for (const language of languages) {
    if (!language.prefix) continue;
    if (clean === language.prefix) return { lang: language.code, path: "/" };
    if (clean.startsWith(`${language.prefix}/`))
      return { lang: language.code, path: clean.slice(language.prefix.length) };
  }
  return { lang: "en", path: clean };
}

export function withLang(lang: LangCode, path: string): string {
  const prefix = languageOf(lang).prefix;
  return path === "/" ? prefix || "/" : `${prefix}${path}`;
}
