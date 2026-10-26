import { indexPath } from "./languages.mjs";

// A small editorial vocabulary keeps tag pages useful and predictable.
export const postTags = ["announcements", "colab", "design", "engineering", "workflow"];

export function validatePostTags(value) {
  if (!Array.isArray(value) || value.length > 3 || new Set(value).size !== value.length)
    throw new Error("Post tags must be an array of at most three unique tags");
  for (const tag of value)
    if (!postTags.includes(tag))
      throw new Error(`Unknown post tag: ${String(tag)}; use ${postTags.join(", ")}`);
}

export const tagPath = (tag, language = "en") => `${indexPath(language)}tags/${tag}/`;
