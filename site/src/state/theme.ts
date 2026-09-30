import { atom } from "jotai";

export type ThemeChoice = "system" | "light" | "dark";
const KEY = "tmt-site-theme";

function read(): ThemeChoice {
  try {
    const stored = localStorage.getItem(KEY);
    return stored === "light" || stored === "dark" ? stored : "system";
  } catch {
    return "system";
  }
}

const base = atom<ThemeChoice>(read());

// The reader's theme: a per-viewer convenience, so browser storage is enough
// and the page still renders when storage is unavailable.
export const themeAtom = atom(
  (get) => get(base),
  (_get, set, next: ThemeChoice) => {
    set(base, next);
    try {
      if (next === "system") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, next);
    } catch {
      // Storage can be blocked; the choice lasts for this visit.
    }
  },
);

// A host page (such as a preview viewer) may already set data-theme for its
// reader. "system" means that choice, or the OS setting when there is none.
const hostTheme = document.documentElement.dataset.theme;

export function applyTheme(choice: ThemeChoice) {
  const theme = choice === "system" ? hostTheme : choice;
  if (theme) document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
}
