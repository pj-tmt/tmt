import { Link, Outlet, useLocation, useNavigate } from "@tanstack/react-router";
import { useAtom } from "jotai";
import { useEffect, useState } from "react";
import { legacyAnchors, pages, windows, type Page } from "../chapters";
import { applyTheme, themeAtom, type ThemeChoice } from "../state/theme";
import { Tag } from "./marks";

export function pageFor(pathname: string): Page {
  const path = pathname.replace(/\/+$/, "") || "/";
  return pages.find((page) => page.path === path) ?? pages[0];
}

// The chapter tree, drawn like `tmux choose-tree`.
function Tree({ current, onPick }: { current: Page; onPick?: () => void }) {
  return (
    <nav aria-label="Chapters" className="font-mono text-xs leading-normal">
      <div className="whitespace-pre text-muted">(0) tmt: {windows.length} windows</div>
      {windows.map((window, w) => {
        const last = w === windows.length - 1;
        const subpages = pages.filter(
          (page) =>
            page.window === window.n && page.path !== window.path && page.crumb.includes(" / "),
        );
        const develop = window.name === "develop";
        const items = develop ? pages.filter((page) => page.window === window.n) : subpages;
        return (
          <div key={window.n}>
            <Link
              to={window.path}
              onClick={onPick}
              className={`block py-0.5 whitespace-pre no-underline ${
                !develop && current.path === window.path
                  ? "font-semibold text-accent"
                  : "text-muted hover:text-text"
              }`}
            >
              {last ? "└─" : "├─"} {window.n}: {window.name}
            </Link>
            {items.map((page, s) => (
              <Link
                key={page.path}
                to={page.path}
                onClick={onPick}
                className={`block py-0.5 whitespace-pre no-underline ${
                  current.path === page.path
                    ? "font-semibold text-accent"
                    : "text-muted hover:text-text"
                }`}
              >
                {last ? "   " : "│  "}
                {s === items.length - 1 ? "└─" : "├─"} {page.crumb.split(" / ")[1]}
              </Link>
            ))}
          </div>
        );
      })}
    </nav>
  );
}

function Toc({ current }: { current: Page }) {
  const [heads, setHeads] = useState<{ id: string; text: string; level: number }[]>([]);
  const [active, setActive] = useState("");
  useEffect(() => {
    const found = [...document.querySelectorAll<HTMLElement>("main h3[id], main h4[id]")].map(
      (head) => ({
        id: head.id,
        text: head.textContent ?? "",
        level: head.tagName === "H4" ? 4 : 3,
      }),
    );
    const spy = () => {
      const line = window.innerHeight * 0.45;
      let on = "";
      for (const head of found) {
        const element = document.getElementById(head.id);
        if (element && element.getBoundingClientRect().top <= line) on = head.id;
      }
      setActive(on);
    };
    // After the new page has painted, so its headings are in the document.
    const frame = requestAnimationFrame(() => {
      setHeads(found);
      spy();
    });
    window.addEventListener("scroll", spy, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("scroll", spy);
    };
  }, [current]);
  if (heads.length < 2) return null;
  return (
    <aside
      aria-label="On this page"
      className="sticky top-0 hidden max-h-[calc(100vh-60px)] self-start overflow-auto pt-10 font-mono text-[12.5px] leading-[1.45] xl:block"
    >
      <div className="mb-2.5 text-[10.5px] font-semibold tracking-[0.08em] text-muted uppercase">
        On this page
      </div>
      {heads.map((head) => (
        <a
          key={head.id}
          href={`#${head.id}`}
          className={`block border-l-2 py-1 no-underline ${head.level === 4 ? "pl-5.5 text-xs" : "pl-2.5"} ${
            active === head.id
              ? "border-accent font-semibold text-accent"
              : "border-rule text-muted hover:text-text"
          }`}
        >
          {head.text}
        </a>
      ))}
    </aside>
  );
}

function Pager({ current }: { current: Page }) {
  const at = pages.indexOf(current);
  const previous = pages[at - 1];
  const next = pages[at + 1];
  const card =
    "flex flex-col gap-1 rounded-md border border-rule bg-sheet px-3.5 py-3 text-text no-underline hover:border-accent";
  const small = "font-mono text-[11px] leading-none tracking-[0.06em] text-muted uppercase";
  return (
    <nav
      aria-label="Page navigation"
      className="mt-14 grid grid-cols-1 gap-3.5 border-t border-rule pt-5 sm:grid-cols-2"
    >
      {previous ? (
        <Link to={previous.path} className={card}>
          <small className={small}>← previous</small>
          <span className="font-display text-[15px] leading-snug font-semibold">
            {previous.title}
          </span>
        </Link>
      ) : (
        <span className="hidden sm:block" />
      )}
      {next && (
        <Link to={next.path} className={`${card} sm:text-right`}>
          <small className={small}>next →</small>
          <span className="font-display text-[15px] leading-snug font-semibold">{next.title}</span>
        </Link>
      )}
    </nav>
  );
}

const THEMES: ThemeChoice[] = ["system", "light", "dark"];

// The bottom bar is a tmux status line: session, windows, then the theme,
// how far you've read and the time.
function StatusBar({ current, onTree }: { current: Page; onTree: () => void }) {
  const [theme, setTheme] = useAtom(themeAtom);
  const [percent, setPercent] = useState(0);
  const [clock, setClock] = useState("--:--");
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
  const edge = "border-t-text/10";
  return (
    <nav
      aria-label="Chapter windows"
      className={`fixed inset-x-0 bottom-0 z-10 flex items-stretch overflow-x-auto border-t ${edge} bg-term pb-[env(safe-area-inset-bottom)] font-mono text-[12.5px] leading-none font-medium whitespace-nowrap text-t-text [scrollbar-width:none]`}
    >
      <i
        className="pointer-events-none absolute top-0 left-0 h-0.5 bg-gradient-to-r from-t-accent to-t-review"
        style={{ width: `${percent}%` }}
      />
      <button
        type="button"
        onClick={onTree}
        aria-label="All chapters"
        className="flex min-h-9 cursor-pointer items-center gap-2 border-r border-t-text/10 bg-transparent px-3.5 font-bold tracking-[0.02em] text-t-accent"
      >
        <svg viewBox="0 0 16 16" aria-hidden="true" className="size-3.5">
          <rect
            x="1.5"
            y="2.5"
            width="13"
            height="11"
            rx="1.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.6"
          />
          <path d="M8 2.5v11M8 8h6.5" stroke="currentColor" strokeWidth="1.6" />
        </svg>
        tmt
      </button>
      <span className="flex items-stretch px-1.5">
        {windows.map((window) => {
          const on = window.n === current.window;
          return (
            <Link
              key={window.n}
              to={window.path}
              aria-current={on ? "page" : undefined}
              className={`relative flex items-center gap-0.5 px-2.5 no-underline transition-colors motion-reduce:transition-none ${
                on ? "font-bold text-t-text" : "text-t-dim hover:text-t-text"
              }`}
            >
              <span className={on ? "text-t-accent" : "opacity-70"}>{window.n}:</span>
              <span className="hidden sm:inline">{window.name}</span>
              {on && (
                <i className="absolute inset-x-2.5 bottom-[7px] h-0.5 rounded-sm bg-t-accent" />
              )}
            </Link>
          );
        })}
      </span>
      <span className="ml-auto flex items-stretch">
        <button
          type="button"
          onClick={() => setTheme(THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length])}
          aria-label={`Theme: ${theme}. Change theme`}
          className="flex cursor-pointer items-center border-l border-t-text/10 bg-transparent px-3.5 text-t-dim hover:text-t-text"
        >
          {theme === "system" ? "◐" : theme === "light" ? "○" : "●"}
          <span className="ml-1.5 hidden sm:inline">{theme === "system" ? "auto" : theme}</span>
        </button>
        <span
          className="hidden min-w-[8ch] items-center justify-end border-l border-t-text/10 px-3.5 text-t-dim tabular-nums sm:flex"
          aria-hidden="true"
        >
          [{percent}%]
        </span>
        <span
          className="hidden items-center border-l border-t-text/10 px-3.5 font-bold tabular-nums sm:flex"
          aria-hidden="true"
        >
          {clock}
        </span>
      </span>
    </nav>
  );
}

export function Layout() {
  const location = useLocation();
  const navigate = useNavigate();
  const current = pageFor(location.pathname);
  const [theme] = useAtom(themeAtom);
  const [treeOpen, setTreeOpen] = useState(false);

  useEffect(() => applyTheme(theme), [theme]);

  // Old single-page links (#squad, #drv-codex, …) land on their new pages.
  useEffect(() => {
    const anchor = location.hash;
    if (current.path === "/" && anchor && legacyAnchors[anchor]) {
      const [to, hash] = legacyAnchors[anchor].split("#");
      void navigate({ to, hash, replace: true });
    }
  }, [current, location.hash, navigate]);

  useEffect(() => {
    document.title = current.path === "/" ? "tmt Handbook" : `${current.title} · tmt Handbook`;
    const target = location.hash && document.getElementById(location.hash);
    if (target) target.scrollIntoView();
    else window.scrollTo(0, 0);
  }, [current, location.hash]);

  useEffect(() => {
    if (!treeOpen) return;
    const close = (event: KeyboardEvent) => event.key === "Escape" && setTreeOpen(false);
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [treeOpen]);

  return (
    <>
      <div className="mx-auto grid max-w-[1080px] grid-cols-1 gap-0 px-4 pb-24 md:grid-cols-[180px_minmax(0,1fr)] md:gap-12 xl:max-w-[1320px] xl:grid-cols-[180px_minmax(0,1fr)_200px]">
        <div className="sticky top-0 hidden self-start pt-10 md:block">
          <Tree current={current} />
        </div>
        <main className="min-w-0">
          <Outlet />
          <Pager current={current} />
        </main>
        <Toc current={current} />
      </div>
      {treeOpen && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label="All chapters"
          className="fixed inset-0 z-20 flex items-end bg-black/40"
          onClick={() => setTreeOpen(false)}
        >
          <div
            className="mb-[calc(36px+env(safe-area-inset-bottom))] w-full border-t border-t-accent bg-term px-4 py-4 text-t-text"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="[&_a]:text-t-dim [&_a:hover]:text-t-text [&_div.text-muted]:text-t-dim">
              <Tree current={current} onPick={() => setTreeOpen(false)} />
            </div>
          </div>
        </div>
      )}
      <StatusBar current={current} onTree={() => setTreeOpen((open) => !open)} />
    </>
  );
}

// The page body: the chapter's border rule and title, then its content.
export function Chapter() {
  const current = pageFor(useLocation().pathname);
  const { Content } = current;
  if (current.path === "/")
    return (
      <section className="pt-14 pb-2">
        <Content />
      </section>
    );
  return (
    <section className="pt-10">
      <div className="flex items-center gap-2.5 font-mono text-xs leading-none text-muted before:w-7 before:border-t before:border-rule after:flex-1 after:border-t after:border-rule">
        <span className="text-text">{current.index}</span>
        <span>{current.crumb}</span>
        {current.status && <Tag kind={current.status.kind}>{current.status.label}</Tag>}
      </div>
      <h2 className="mt-4.5 mb-3.5 font-display text-[clamp(28px,4vw,40px)] leading-[1.08] font-extrabold tracking-[-0.015em] text-balance">
        {current.title}
      </h2>
      <Content />
    </section>
  );
}
