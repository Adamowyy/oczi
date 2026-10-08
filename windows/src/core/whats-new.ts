// Which releases get a card the first time they run, text from the i18n tables.

import type { TextKey } from "./i18n";

const NEWS: Record<string, TextKey> = {
  "0.1.4": "news.0.1.4",
  "0.1.6": "news.0.1.6",
  "0.1.7": "news.0.1.7",
  "0.2.0": "news.0.2.0",
};

/** The card's body for a version, or null when there is nothing to say. */
export function newsKey(version: string): TextKey | null {
  return NEWS[version] ?? null;
}
