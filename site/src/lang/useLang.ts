import { useLocation } from "@tanstack/react-router";
import { splitLang } from "./languages";

// The language and shared page path of the current URL.
export function useLang() {
  return splitLang(useLocation().pathname);
}
