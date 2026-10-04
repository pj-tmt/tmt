import { Outlet, useLocation, useNavigate } from "@tanstack/react-router";
import { useAtom } from "jotai";
import { useEffect, useRef, useState } from "react";
import { legacyAnchors, pages, type Page } from "../chapters";
import { languageOf, withLang } from "../lang/languages";
import { readLangPreference } from "../lang/preference";
import { english } from "../lang/strings";
import { localize } from "../lang/translations";
import { useLang } from "../lang/useLang";
import { useStrings } from "../lang/useStrings";
import { applyTheme, themeAtom } from "../state/theme";
import { LocalLink } from "./LocalLink";
import { Tag } from "./marks";
import { StatusBar } from "./StatusBar";

// The page for a language-free path (see splitLang), or the home page.
export function pageFor(path: string): Page {
  return pages.find((page) => page.path === path) ?? pages[0];
}

function useCurrent() {
  const { lang, path } = useLang();
  const current = pageFor(path);
  return { lang, current, ...localize(lang, current) };
}

function Toc({ current }: { current: Page }) {
  const { ui } = useStrings();
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
      aria-label={ui.onThisPage}
      className="sticky top-9 hidden max-h-[calc(100vh-36px)] self-start overflow-auto pt-10 font-mono text-[12.5px] leading-[1.45] xl:block"
    >
      <div className="mb-2.5 text-[10.5px] font-semibold tracking-[0.08em] text-muted uppercase">
        {ui.onThisPage}
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
  const { ui } = useStrings();
  const { lang } = useLang();
  const at = pages.indexOf(current);
  const previous = pages[at - 1];
  const next = pages[at + 1];
  const card =
    "flex flex-col gap-1 border border-rule bg-sheet px-3.5 py-3 text-text no-underline hover:border-accent";
  const small = "font-mono text-[11px] leading-none tracking-[0.06em] text-muted uppercase";
  return (
    <nav
      aria-label={ui.pageNavigation}
      className="mt-14 grid grid-cols-1 gap-3.5 border-t border-rule pt-5 sm:grid-cols-2"
    >
      {previous ? (
        <LocalLink to={previous.path} className={card}>
          <small className={small}>{ui.previous}</small>
          <span className="font-display text-[15px] leading-snug font-semibold">
            {localize(lang, previous).title}
          </span>
        </LocalLink>
      ) : (
        <span className="hidden sm:block" />
      )}
      {next && (
        <LocalLink to={next.path} className={`${card} sm:text-right`}>
          <small className={small}>{ui.next}</small>
          <span className="font-display text-[15px] leading-snug font-semibold">
            {localize(lang, next).title}
          </span>
        </LocalLink>
      )}
    </nav>
  );
}

export function Layout() {
  const location = useLocation();
  const navigate = useNavigate();
  const { lang, current, title, translated } = useCurrent();
  const [theme] = useAtom(themeAtom);

  useEffect(() => applyTheme(theme), [theme]);
  useEffect(() => {
    document.documentElement.lang = languageOf(lang).htmlLang;
  }, [lang]);

  // A reader who chose a language before lands in it when opening an English
  // address. Only the first load does this, so choosing EN afterwards sticks.
  const arrived = useRef(false);
  useEffect(() => {
    if (arrived.current) return;
    arrived.current = true;
    const saved = readLangPreference();
    if (lang === "en" && saved && saved !== "en")
      void navigate({
        to: withLang(saved, current.path),
        hash: location.hash || undefined,
        replace: true,
      });
  }, [lang, current, location.hash, navigate]);

  // Old single-page links (#squad, #drv-codex, …) land on their new pages.
  useEffect(() => {
    const anchor = location.hash;
    if (current.path === "/" && anchor && legacyAnchors[anchor]) {
      const [to, hash] = legacyAnchors[anchor].split("#");
      void navigate({ to: withLang(lang, to), hash, replace: true });
    }
  }, [current, lang, location.hash, navigate]);

  useEffect(() => {
    // The home page is plain "tmt Handbook" in English; a translated one
    // (or any other page) names itself by its own title.
    const named = current.path !== "/" || (lang !== "en" && translated);
    document.title = named ? `${title} · tmt Handbook` : "tmt Handbook";
    const target = location.hash && document.getElementById(location.hash);
    if (target) target.scrollIntoView();
    else window.scrollTo(0, 0);
  }, [current, title, lang, translated, location.hash]);

  return (
    <>
      <StatusBar current={current} />
      {current.path === "/" ? (
        // Home is one wide, edge-to-edge tour: no contents column, no pager.
        <main className="mx-auto max-w-[1180px] min-w-0 px-4 pb-16">
          <Outlet />
        </main>
      ) : (
        <div className="mx-auto grid max-w-[900px] grid-cols-1 px-4 pb-16 xl:max-w-[1120px] xl:grid-cols-[minmax(0,860px)_200px] xl:gap-12">
          <main className="min-w-0">
            <Outlet />
            <Pager current={current} />
          </main>
          <Toc current={current} />
        </div>
      )}
    </>
  );
}

// The page body: the chapter's border rule and title, then its content.
export function Chapter() {
  const { lang, current, title, Content, translated } = useCurrent();
  const { chrome } = useStrings();
  const crumbs: Record<string, string> = chrome.crumbs;
  const statusLabels: Record<string, string> = chrome.status;
  const fallback = lang !== "en" && !translated;
  const note = fallback && <NotTranslated />;
  if (current.path === "/")
    return (
      <section className="pt-14 pb-2">
        {note}
        <div lang={fallback ? "en" : undefined}>
          <Content />
        </div>
      </section>
    );
  return (
    <section className="pt-10">
      <div className="flex items-center gap-2.5 font-mono text-xs leading-none text-muted before:w-7 before:border-t before:border-rule after:flex-1 after:border-t after:border-rule">
        <span className="text-text">{current.index}</span>
        <span>{crumbs[current.file] ?? current.crumb}</span>
        {current.status && (
          <Tag kind={current.status.kind}>
            {statusLabels[current.status.kind] ?? current.status.label}
          </Tag>
        )}
      </div>
      <h2 className="mt-4.5 mb-3.5 font-mono text-[clamp(26px,3.6vw,40px)] leading-[1.08] font-bold tracking-[-0.02em] text-balance">
        {title}
      </h2>
      {note}
      <div lang={fallback ? "en" : undefined}>
        <Content />
      </div>
    </section>
  );
}

// Shown above an English fallback page until its translation lands.
function NotTranslated() {
  const { ui } = useStrings();
  return (
    <p
      lang={ui.notTranslated === english.ui.notTranslated ? "en" : undefined}
      className="mb-5 border border-rule bg-sheet px-3.5 py-2.5 font-mono text-[13px] leading-normal text-muted"
    >
      {ui.notTranslated}
    </p>
  );
}
