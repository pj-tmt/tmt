import { Link, useLocation } from "@tanstack/react-router";
import { languages, withLang } from "../lang/languages";
import { saveLangPreference } from "../lang/preference";
import { useLang } from "../lang/useLang";
import { useStrings } from "../lang/useStrings";

// EN · 日本語 · 中文: each is a real link to the same page in that language,
// and choosing one is remembered for the next visit. The status bar shows it
// from the small breakpoint up; below that it sits at the top of the tree menu.
export function LanguageSwitcher({
  variant,
  onPick,
}: {
  variant: "bar" | "menu";
  onPick?: () => void;
}) {
  const { lang, path } = useLang();
  const { ui } = useStrings();
  const hash = useLocation().hash;
  const bar = variant === "bar";
  return (
    <nav aria-label={ui.language} className="flex items-stretch">
      {languages.map((language) => {
        const on = language.code === lang;
        return (
          <Link
            key={language.code}
            to={withLang(language.code, path)}
            hash={hash || undefined}
            lang={language.htmlLang}
            hrefLang={language.htmlLang}
            activeOptions={{ exact: true }}
            onClick={() => {
              saveLangPreference(language.code);
              onPick?.();
            }}
            className={
              bar
                ? `flex items-center px-2 no-underline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-t-waiting ${
                    on ? "bg-term-bar text-t-accent" : "text-paper hover:bg-term-bar/15"
                  }`
                : `px-2.5 py-1.5 font-mono text-[13px] no-underline ${
                    on ? "font-semibold text-t-accent" : "text-t-dim hover:text-t-text"
                  }`
            }
          >
            {language.label}
          </Link>
        );
      })}
    </nav>
  );
}
