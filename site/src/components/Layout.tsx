import { Link, Outlet, useLocation, useNavigate } from "@tanstack/react-router";
import { useAtom } from "jotai";
import { useEffect, useState } from "react";
import { legacyAnchors, pages, type Page } from "../chapters";
import { applyTheme, themeAtom } from "../state/theme";
import { Tag } from "./marks";
import { StatusBar } from "./StatusBar";

export function pageFor(pathname: string): Page {
  const path = pathname.replace(/\/+$/, "") || "/";
  return pages.find((page) => page.path === path) ?? pages[0];
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
      className="sticky top-9 hidden max-h-[calc(100vh-36px)] self-start overflow-auto pt-10 font-mono text-[12.5px] leading-[1.45] xl:block"
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

export function Layout() {
  const location = useLocation();
  const navigate = useNavigate();
  const current = pageFor(location.pathname);
  const [theme] = useAtom(themeAtom);

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

  return (
    <>
      <StatusBar current={current} />
      <div className="mx-auto grid max-w-[900px] grid-cols-1 px-4 pb-16 xl:max-w-[1120px] xl:grid-cols-[minmax(0,860px)_200px] xl:gap-12">
        <main className="min-w-0">
          <Outlet />
          <Pager current={current} />
        </main>
        <Toc current={current} />
      </div>
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
      <h2 className="mt-4.5 mb-3.5 font-mono text-[clamp(26px,3.6vw,40px)] leading-[1.08] font-bold tracking-[-0.02em] text-balance">
        {current.title}
      </h2>
      <Content />
    </section>
  );
}
