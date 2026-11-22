// The chat providers Oczi can answer through, mirroring `src-tauri/src/providers.rs`.
// The Rust table is what validates a stored model; this one is what the settings window
// offers and what the island labels its key badge with, so the two have to stay in step.

import type { TextKey } from "./i18n";

export interface ChatProvider {
  /** Stored in `settings.provider`, and what `providers::get` looks up on the Rust side. */
  id: string;
  /** Brand name: identical in every language, so it is not an i18n key. */
  label: string;
  /** Where the key lives in the Windows Credential Manager. */
  key: string;
  /** What a key looks like, shown as the field's placeholder. */
  placeholder: string;
  /** Where the user gets one — the settings section shows this as plain text. */
  keyUrl: string;
  /** Models in offer order. The first is the provider's default, and belongs to it alone. */
  models: [string, string][];
  /** A word shown in brackets after the name in the provider picker, for what costs nothing —
   *  the one thing that decides the choice for a user who does not want to pay. */
  note?: TextKey;
}

export const CHAT_PROVIDERS: ChatProvider[] = [
  {
    id: "deepseek",
    label: "DeepSeek",
    key: "deepseek-api-key",
    placeholder: "sk-…",
    keyUrl: "https://platform.deepseek.com/api_keys",
    // Flash only, as in the Rust table: the bigger model is a second choice with nothing to
    // show for it in a card this small.
    models: [["deepseek-flash", "DeepSeek Flash"]],
  },
  {
    id: "gemini",
    label: "Gemini",
    key: "gemini-api-key",
    placeholder: "AIza…",
    keyUrl: "https://aistudio.google.com/apikey",
    // The free tier is 20 requests a day on the big models and 500 on the Lite ones, and a turn
    // is two to four requests — so the Lite models are the everyday pair and 3.6-flash stays in the
    // Rust table only as a last resort in the walk.
    models: [
      ["gemini-3.5-flash-lite", "Gemini 3.5 Flash Lite"],
      ["gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite"],
    ],
    note: "set.freeTier",
  },
];

/** The provider for an id, falling back to the first — an unknown value in settings.json
 *  is what `providers::get` also falls back on, so both sides agree. */
export function chatProvider(id: string): ChatProvider {
  return CHAT_PROVIDERS.find((provider) => provider.id === id) ?? CHAT_PROVIDERS[0];
}

/** The credential entry name, for `secretPresent` / `secretSet` / `secretClear`. */
export function chatKeyFor(id: string): string {
  return chatProvider(id).key;
}

/** The name to put in front of the user: the island's key badge and the ask button. */
export function chatLabel(id: string): string {
  return chatProvider(id).label;
}
