import { Outlet, useLocation, useNavigate } from "@tanstack/react-router";
import { useAtom } from "jotai";
import { useEffect, useRef } from "react";
import { languageOf, withLang } from "../lang/languages";
import { readLangPreference } from "../lang/preference";
import { useLang } from "../lang/useLang";
import { applyTheme, themeAtom } from "../state/theme";

// Home and the Colab landing share theme and navigation effects. Retained MDX stays source-only.
export function Layout() {
  const location = useLocation();
  const navigate = useNavigate();
  const { lang } = useLang();
  const [theme] = useAtom(themeAtom);
  const arrived = useRef(false);
  const isColab = location.pathname.replace(/\/+$/, "") === "/colab";

  useEffect(() => applyTheme(theme), [theme]);
  useEffect(() => {
    document.documentElement.lang = languageOf(lang).htmlLang;
    document.title = isColab ? "Colab · Terminal Tunnel" : "Terminal Tunnel";
  }, [lang, isColab]);
  useEffect(() => {
    if (arrived.current) return;
    arrived.current = true;
    const saved = readLangPreference();
    if (!isColab && lang === "en" && saved && saved !== "en")
      void navigate({ to: withLang(saved, "/"), hash: location.hash, replace: true });
  }, [lang, isColab, location.hash, navigate]);
  useEffect(() => {
    const target = location.hash && document.getElementById(location.hash);
    if (target) target.scrollIntoView();
    else window.scrollTo(0, 0);
  }, [lang, location.hash, location.pathname]);
  return <Outlet />;
}
