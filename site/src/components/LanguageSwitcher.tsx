import { Link, useLocation } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState } from "react";
import { languageOf, languages, withLang } from "../lang/languages";
import { saveLangPreference } from "../lang/preference";
import { useLang } from "../lang/useLang";
import { useStrings } from "../lang/useStrings";

// A disclosure keeps ordinary links, including opening a language in a new tab.
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
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const id = useId();
  const bar = variant === "bar";

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !container.current?.contains(event.target))
        setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  return (
    <div
      ref={container}
      className="relative flex items-stretch"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.stopPropagation();
          setOpen(false);
          trigger.current?.focus();
        }
      }}
    >
      <button
        ref={trigger}
        type="button"
        aria-label={`${ui.language}: ${languageOf(lang).label}`}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
        className={`flex cursor-pointer items-center gap-2 px-2.5 py-2 font-mono text-[13px] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-t-waiting ${
          bar ? "text-paper hover:bg-term-bar/15" : "text-t-text hover:bg-term-bar"
        }`}
      >
        {languageOf(lang).label}
        <span aria-hidden="true" className="text-[10px]">
          {open ? "▴" : "▾"}
        </span>
      </button>
      <ul
        id={id}
        hidden={!open}
        aria-label={ui.language}
        className={`absolute top-full z-20 m-0 min-w-36 list-none border border-term-edge bg-term p-1 font-mono text-[13px] leading-normal font-normal ${bar ? "right-0" : "left-0"}`}
      >
        {languages.map((language) => (
          <li key={language.code}>
            <Link
              to={withLang(language.code, path)}
              hash={hash || undefined}
              lang={language.htmlLang}
              hrefLang={language.htmlLang}
              aria-current={language.code === lang ? "true" : undefined}
              onClick={() => {
                saveLangPreference(language.code);
                setOpen(false);
                onPick?.();
              }}
              className={`flex items-center justify-between gap-4 px-2.5 py-2 no-underline hover:bg-term-bar focus-visible:outline-t-waiting ${
                language.code === lang ? "font-semibold text-t-accent" : "text-t-text"
              }`}
            >
              {language.label}
              <span aria-hidden="true">{language.code === lang ? "✓" : ""}</span>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
