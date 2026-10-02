import { useAtom } from "jotai";
import { useEffect, useState } from "react";
import { windows, type Page } from "../chapters";
import { themeAtom, type ThemeChoice } from "../state/theme";
import { ChapterTree } from "./ChapterTree";
import { LanguageSwitcher } from "./LanguageSwitcher";
import { LocalLink } from "./LocalLink";

const THEMES: ThemeChoice[] = ["system", "light", "dark"];

// The bar is always the dark terminal look, in either theme: accent fill with
// the terminal bar color as ink, and the active window inverted, as in tmux.
const focus =
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-t-waiting";

// The top bar is a tmux status line and the site's chapter navigation: session,
// windows, then the theme, how far you've read and the time. The session name
// opens the choose-tree of every page.
export function StatusBar({ current }: { current: Page }) {
  const [theme, setTheme] = useAtom(themeAtom);
  const [percent, setPercent] = useState(0);
  const [clock, setClock] = useState("--:--");
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    const read = () => {
      const height = document.documentElement.scrollHeight - window.innerHeight;
      setPercent(height > 0 ? Math.min(100, Math.round((window.scrollY / height) * 100)) : 0);
    };
    const tick = () => {
      const now = new Date();
      setClock(
        `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`,
      );
    };
    read();
    tick();
    const timer = setInterval(tick, 15000);
    window.addEventListener("scroll", read, { passive: true });
    window.addEventListener("resize", read);
    return () => {
      clearInterval(timer);
      window.removeEventListener("scroll", read);
      window.removeEventListener("resize", read);
    };
  }, [current]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = (event: KeyboardEvent) => event.key === "Escape" && setMenuOpen(false);
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [menuOpen]);

  return (
    <header className="sticky top-0 z-10 pt-[env(safe-area-inset-top)] bg-t-accent">
      <nav
        aria-label="Chapter windows"
        className="relative flex items-stretch overflow-x-auto bg-t-accent font-mono text-[13px] leading-none font-semibold whitespace-nowrap text-term-bar [scrollbar-width:none]"
      >
        <button
          type="button"
          onClick={() => setMenuOpen((open) => !open)}
          aria-label="All chapters"
          aria-expanded={menuOpen}
          aria-controls="chapter-menu"
          className={`flex min-h-9 cursor-pointer items-center gap-1.5 bg-term-bar px-2.5 text-t-accent sm:px-3 ${focus}`}
        >
          [tmt]
          <span aria-hidden="true" className="text-[10px]">
            {menuOpen ? "▴" : "▾"}
          </span>
        </button>
        {windows.map((window) => {
          const on = window.n === current.window;
          return (
            <LocalLink
              key={window.n}
              to={window.path}
              aria-current={on ? "page" : undefined}
              className={`flex items-center px-2.5 no-underline sm:px-3 ${focus} ${
                on ? "bg-term-bar text-t-accent" : "text-term-bar hover:bg-term-bar/15"
              }`}
            >
              {window.n}
              <span className="hidden sm:inline">:{window.name}</span>
              <span className="sm:hidden">:</span>
            </LocalLink>
          );
        })}
        <span className="ml-auto flex items-stretch">
          <span className="hidden sm:flex">
            <LanguageSwitcher variant="bar" />
          </span>
          <button
            type="button"
            onClick={() => setTheme(THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length])}
            aria-label={`Theme: ${theme}. Change theme`}
            className={`flex cursor-pointer items-center px-2.5 text-term-bar sm:px-3 hover:bg-term-bar/15 ${focus}`}
          >
            {theme === "system" ? "◐" : theme === "light" ? "○" : "●"}
            <span className="ml-1.5 hidden sm:inline">{theme === "system" ? "auto" : theme}</span>
          </button>
          <span
            className="hidden min-w-[8ch] items-center justify-end px-3 tabular-nums sm:flex"
            aria-hidden="true"
          >
            [{percent}%]
          </span>
          <span className="hidden items-center px-3 tabular-nums sm:flex" aria-hidden="true">
            {clock}
          </span>
        </span>
      </nav>
      {menuOpen && (
        <>
          <button
            type="button"
            aria-label="Close menu"
            tabIndex={-1}
            onClick={() => setMenuOpen(false)}
            className="fixed inset-0 -z-10 cursor-default bg-black/40"
          />
          <div
            id="chapter-menu"
            className="absolute top-full left-0 max-h-[calc(100dvh-48px)] w-max max-w-full min-w-64 overflow-y-auto border-r border-b border-term-edge bg-term px-4 py-3 text-t-text"
          >
            <div className="mb-2 border-b border-term-edge pb-2 sm:hidden">
              <LanguageSwitcher variant="menu" onPick={() => setMenuOpen(false)} />
            </div>
            <ChapterTree current={current} onPick={() => setMenuOpen(false)} />
          </div>
        </>
      )}
    </header>
  );
}
