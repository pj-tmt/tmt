import { languages, type LangCode } from "./languages";

const KEY = "tmt-site-lang";

// The reader's language: a per-browser convenience, so browser storage is
// enough and every page still works when storage is unavailable.
export function readLangPreference(): LangCode | null {
  try {
    const stored = localStorage.getItem(KEY);
    return languages.find((language) => language.code === stored)?.code ?? null;
  } catch {
    return null;
  }
}

export function saveLangPreference(lang: LangCode) {
  try {
    localStorage.setItem(KEY, lang);
  } catch {
    // Storage can be blocked; the choice lasts for this visit.
  }
}
