import { useLocation, useNavigate } from "@tanstack/react-router";
import { useAtom } from "jotai";
import { useEffect, useRef } from "react";
import { HomePage } from "../home/HomePage";
import { languageOf, withLang } from "../lang/languages";
import { readLangPreference } from "../lang/preference";
import { useLang } from "../lang/useLang";
import { applyTheme, themeAtom } from "../state/theme";

// The public site is Home only. Retained MDX is not part of this render graph.
export function Layout() {
  const location = useLocation();
  const navigate = useNavigate();
  const { lang } = useLang();
  const [theme] = useAtom(themeAtom);
  const arrived = useRef(false);

  useEffect(() => applyTheme(theme), [theme]);
  useEffect(() => {
    document.documentElement.lang = languageOf(lang).htmlLang;
    document.title = "Terminal Tunnel";
  }, [lang]);
  useEffect(() => {
    if (arrived.current) return;
    arrived.current = true;
    const saved = readLangPreference();
    if (lang === "en" && saved && saved !== "en")
      void navigate({ to: withLang(saved, "/"), hash: location.hash, replace: true });
  }, [lang, location.hash, navigate]);
  useEffect(() => {
    const target = location.hash && document.getElementById(location.hash);
    if (target) target.scrollIntoView();
    else window.scrollTo(0, 0);
  }, [lang, location.hash]);
  return <HomePage />;
}
