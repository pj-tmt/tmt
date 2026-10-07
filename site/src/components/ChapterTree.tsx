import { pages, windows, type Page } from "../chapters";
import { localize } from "../lang/translations";
import { useLang } from "../lang/useLang";
import { useStrings } from "../lang/useStrings";
import { LocalLink } from "./LocalLink";

// Full localized names wrap at narrow widths; selection and focus stay distinct.
export function ChapterTree({ current, onPick }: { current: Page; onPick: () => void }) {
  const { ui, chrome } = useStrings();
  const { lang } = useLang();
  return (
    <nav aria-label={ui.allChapters} className="chapter-tree">
      {windows.map((window) => (
        <div key={window.n} className="chapter-tree-group">
          <div className="chapter-tree-label">
            {window.n} / {chrome.crumbs[window.name as keyof typeof chrome.crumbs] ?? window.name}
          </div>
          {pages
            .filter((page) => page.window === window.n)
            .map((page) => (
              <LocalLink
                key={page.path}
                to={page.path}
                activeOptions={{ exact: true }}
                onClick={onPick}
                aria-current={current.path === page.path ? "page" : undefined}
                className="chapter-tree-link"
              >
                <span aria-hidden="true">{current.path === page.path ? "▸" : ""}</span>
                <span>{localize(lang, page).title}</span>
              </LocalLink>
            ))}
        </div>
      ))}
    </nav>
  );
}
