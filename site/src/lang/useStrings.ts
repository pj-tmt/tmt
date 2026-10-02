import { useMemo } from "react";
import { english, overlay, type Strings } from "./strings";
import { useLang } from "./useLang";

const overrides = import.meta.glob<unknown>("../i18n/*/strings.json", {
  eager: true,
  import: "default",
});

export function useStrings(): Strings {
  const { lang } = useLang();
  return useMemo(
    () => overlay<Strings>(english, overrides[`../i18n/${lang}/strings.json`]),
    [lang],
  );
}
