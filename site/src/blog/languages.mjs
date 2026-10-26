// Metadata and directory names stay lowercase; HTML/RSS use canonical BCP 47 tags.
export const languages = [
  { id: "en", tag: "en", label: "English" },
  { id: "ja", tag: "ja", label: "Japanese" },
  { id: "zh-hant", tag: "zh-Hant", label: "Traditional Chinese" },
  { id: "zh-hans", tag: "zh-Hans", label: "Simplified Chinese" },
];

export function languageInfo(id) {
  const language = languages.find((item) => item.id === id);
  if (!language) throw new Error("Unsupported post language: use en, ja, zh-hant or zh-hans");
  return language;
}

export const indexPath = (language) => (language === "en" ? "" : languageInfo(language).id + "/");
export const postPath = (slug, language) => `${indexPath(language)}posts/${slug}/`;
export const postDirectory = ({ slug, language }) =>
  slug + (language === "en" ? "" : "/" + languageInfo(language).id);
