import { Link } from "@tanstack/react-router";
import { pages, windows, type Page } from "../chapters";

// The chapter tree, drawn like `tmux choose-tree`. It lives in the status
// bar's menu, which is always dark like the bar, so it uses the terminal colors.
export function ChapterTree({ current, onPick }: { current: Page; onPick: () => void }) {
  const row = "block py-1 whitespace-pre no-underline";
  const tone = (on: boolean) =>
    on ? "font-semibold text-t-accent" : "text-t-dim hover:text-t-text";
  return (
    <nav aria-label="All chapters" className="font-mono text-[13px] leading-normal">
      <div className="whitespace-pre text-t-dim">(0) tmt: {windows.length} windows</div>
      {windows.map((window, w) => {
        const last = w === windows.length - 1;
        const develop = window.name === "develop";
        const items = develop
          ? pages.filter((page) => page.window === window.n)
          : pages.filter(
              (page) =>
                page.window === window.n && page.path !== window.path && page.crumb.includes(" / "),
            );
        return (
          <div key={window.n}>
            <Link
              to={window.path}
              onClick={onPick}
              className={`${row} ${tone(!develop && current.path === window.path)}`}
            >
              {last ? "└─" : "├─"} {window.n}: {window.name}
            </Link>
            {items.map((page, s) => (
              <Link
                key={page.path}
                to={page.path}
                onClick={onPick}
                className={`${row} ${tone(current.path === page.path)}`}
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
