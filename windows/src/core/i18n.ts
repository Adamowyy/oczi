
export type Lang = "en" | "pl";

export const LANGUAGES: { tag: Lang; label: string }[] = [
  { tag: "en", label: "English" },
  { tag: "pl", label: "Polski" },
];

const EN = {
  // Settings window
  "set.title": "Oczi — Settings",
  "set.deepseek": "DeepSeek",
  "set.keyStored": "Key saved in the Windows Credential Manager.",
  "set.keyMissing": "No key yet — the chat needs one.",
  "set.keyPlaceholderSaved": "••••••••••••  (saved)",
  "set.keySave": "Save key",
  "set.keyRemove": "Remove",
  "set.keySaved": "Saved. It never touches the disk.",
  "set.keySaveFailed": "Could not save: {}",
  "set.keyRemoved": "Key removed.",
  "set.keyRemoveFailed": "Could not remove: {}",
  "set.apiKey": "API key",
  "set.model": "Model",
  "set.thinking": "Thinking",
  "set.thinkingHint": "slower, but it reasons first",
  "set.noKeyNeeded": "no key needed",
  "set.needsKey": "needs a key",
  "set.braveKey": "Brave key",
  "set.tavilyKey": "Tavily key",
  "set.save": "Save",
  "set.stored": "••••••••  (stored)",
  "set.internet": "Internet",
  "set.webSearch": "Web search",
  "set.webSearchHint": "fresh data, links, weather, prices",
  "set.searchEngine": "Search engine",
  "set.searchKeysHint": "keys only for Brave / Tavily — DuckDuckGo works out of the box",
  "set.integrations": "Integrations",
  "set.integrationsNote": "Pick up to {} pills next to Iskra — {} of {} used. Keys are stored in the Windows Credential Manager, never on disk.",
  "set.secretKey": "Secret key",
  "set.token": "Token",
  "set.instanceUrl": "Instance URL",
  "set.integrationToken": "Integration token",
  "tab.home": "Home",
  "tab.ask": "Ask",
  "tab.add": "Add a file",
  "set.general": "General",
  "set.autoCloseChip": "Auto-close · {}s",
  "set.language": "Language",
  "set.autoClose": "Auto-close",
  "set.autoCloseHint": "seconds after you leave the island",
  "set.islandScreen": "Island lives on",
  "set.screenPrimary": "Main display",
  "set.screenCursor": "Display with the cursor",
  "set.hotkey": "Summon hotkey",
  "set.autostart": "Start with Windows",
  "set.privacy": "No telemetry. Network requests go only to the services you configure yourself.",

  // Island
  "chat.placeholder": "Ask me anything…",
  "chat.placeholderSnip": "Ask about this screenshot…",
  "chat.send": "Send",
  "chat.askButton": "Ask DeepSeek",
  "chat.askSub": "Ask me anything.",
  "chat.openBar": "Open the chat",
  "int.missingKey": "No key",
  "int.open": "Open {}",
  "int.openN8n": "Open n8n",
  "int.settings": "Settings…",
  "int.tip": "Settings",
  "int.tipOpen": "Open",
  "int.done": "Done",
  "int.cancelled": "Cancelled",
  "int.failed": "Error",
  "int.noMeetings": "No upcoming meetings",
  "int.finishedOk": "Finished successfully.",
  "int.noDetails": "No error details.",
  "absence.text": "Give me a moment — I'll be back at work in three seconds.",
  "upload.ask": "Ask about it",
  "upload.cancel": "Cancel",
  "snip.hint": "Select the area Iskra should look at · Esc cancels",
  "chat.snipTip": "Snip part of the screen and ask about it",
  "chat.snipHint": "Screenshot: Ctrl+Alt+Shift+S",
  "chat.snipLabel": "Screenshot {}×{}",
  "upload.fileLower": "file",
  "upload.file": "File",
  "chat.newTip": "New chat — clear this conversation",
  "upload.dropHere": "Drop files here",
  "upload.whatToDo": "What should I do with it?",
  "int.connected": "Connected · loading…",
  "int.refresh": "Refresh",
  "int.details": "Details",
  "int.deployments": "Deployments",
  "int.deployment": "Deployment",
  "int.overview": "Overview",
  "int.starsTotal": "Stars in total",
  "int.payment": "Payment",
  "int.payments": "Payments",
  "int.untitled": "Untitled",
  "int.workflow": "Workflow",
  "time.justNow": "just now",
  "empty.quiet": "Nothing happening right now.",
  "empty.tooMuch": "Too much at once.",
  "empty.noEmail": "Sending by email is not in this version.",
} as const;

export type TextKey = keyof typeof EN;

const PL: Record<TextKey, string> = {
  "set.title": "Oczi — Ustawienia",
  "set.deepseek": "DeepSeek",
  "set.keyStored": "Klucz zapisany w menedżerze poświadczeń Windows.",
  "set.keyMissing": "Brak klucza — czat go potrzebuje.",
  "set.keyPlaceholderSaved": "••••••••••••  (zapisany)",
  "set.keySave": "Zapisz klucz",
  "set.keyRemove": "Usuń",
  "set.keySaved": "Zapisano. Nigdy nie trafia na dysk.",
  "set.keySaveFailed": "Nie udało się zapisać: {}",
  "set.keyRemoved": "Klucz usunięty.",
  "set.keyRemoveFailed": "Nie udało się usunąć: {}",
  "set.apiKey": "Klucz API",
  "set.model": "Model",
  "set.thinking": "Myślenie",
  "set.thinkingHint": "wolniej, ale najpierw myśli",
  "set.noKeyNeeded": "bez klucza",
  "set.needsKey": "wymaga klucza",
  "set.braveKey": "Klucz Brave",
  "set.tavilyKey": "Klucz Tavily",
  "set.save": "Zapisz",
  "set.stored": "••••••••  (zapisany)",
  "set.internet": "Internet",
  "set.webSearch": "Szukanie w sieci",
  "set.webSearchHint": "świeże dane, linki, pogoda, ceny",
  "set.searchEngine": "Wyszukiwarka",
  "set.searchKeysHint": "klucze tylko dla Brave / Tavily — DuckDuckGo działa od razu",
  "set.integrations": "Integracje",
  "set.integrationsNote": "Maksymalnie {} pigułki obok Iskry — użyto {}/{} używanych. Klucze są przechowywane w menedżerze poświadczeń Windows, nigdy na dysku.",
  "set.secretKey": "Klucz tajny",
  "set.token": "Token",
  "set.instanceUrl": "URL instancji",
  "set.integrationToken": "Token integracji",
  "tab.home": "Główna",
  "tab.ask": "Pytaj",
  "tab.add": "Dodaj plik",
  "set.general": "Ogólne",
  "set.autoCloseChip": "Auto-zamykanie · {}s",
  "set.language": "Język",
  "set.autoClose": "Auto-zamykanie",
  "set.autoCloseHint": "sekund po opuszczeniu wyspy",
  "set.islandScreen": "Wyspa mieszka na",
  "set.screenPrimary": "Monitor główny",
  "set.screenCursor": "Monitor pod kursorem",
  "set.hotkey": "Skrót otwierający",
  "set.autostart": "Uruchamiaj przy starcie",
  "set.privacy": "Bez telemetrii. Zapytania sieciowe trafiają wyłącznie do usług, które sam konfigurujesz.",

  "chat.placeholder": "Zapytaj mnie o cokolwiek…",
  "chat.placeholderSnip": "Zapytaj o ten zrzut ekranu…",
  "chat.send": "Wyślij",
  "chat.askButton": "Zapytaj DeepSeek",
  "chat.askSub": "Zapytaj mnie o cokolwiek.",
  "chat.openBar": "Otwórz czat",
  "int.missingKey": "Brak klucza",
  "int.open": "Otwórz {}",
  "int.openN8n": "Otwórz n8n",
  "int.settings": "Ustawienia…",
  "int.tip": "Ustawienia",
  "int.tipOpen": "Otwórz",
  "int.done": "Gotowe",
  "int.cancelled": "Anulowane",
  "int.failed": "Błąd",
  "int.noMeetings": "Brak zaplanowanych spotkań",
  "int.finishedOk": "Ukończono pomyślnie.",
  "int.noDetails": "Brak szczegółów błędu.",
  "absence.text": "Daj mi chwilę — za trzy sekundy wracam do pracy.",
  "upload.ask": "Zadaj pytanie na jego temat",
  "upload.cancel": "Anuluj",
  "snip.hint": "Zaznacz obszar, na który Iskra ma spojrzeć · Esc anuluje",
  "chat.snipTip": "Zrób zrzut fragmentu ekranu i zapytaj o niego",
  "chat.snipHint": "Zrzut ekranu: Ctrl+Alt+Shift+S",
  "chat.snipLabel": "Zrzut ekranu {}×{}",
  "upload.fileLower": "plik",
  "upload.file": "Plik",
  "chat.newTip": "Nowy czat — wyczyść tę rozmowę",
  "upload.dropHere": "Upuść pliki tutaj",
  "upload.whatToDo": "Co chcesz z nim zrobić?",
  "int.connected": "Połączono · wczytywanie…",
  "int.refresh": "Odśwież",
  "int.details": "Szczegóły",
  "int.deployments": "Wdrożenia",
  "int.deployment": "Wdrożenie",
  "int.overview": "Przegląd",
  "int.starsTotal": "Łącznie gwiazdek",
  "int.payment": "Płatność",
  "int.payments": "Płatności",
  "int.untitled": "Bez tytułu",
  "int.workflow": "Przepływ",
  "time.justNow": "przed chwilą",
  "empty.quiet": "Na razie nic się nie dzieje.",
  "empty.tooMuch": "Za dużo naraz.",
  "empty.noEmail": "Wysyłanie e-mailem nie jest dostępne w tej wersji.",
};

const TABLES: Record<Lang, Record<TextKey, string>> = { en: EN, pl: PL };

let lang: Lang = "en";

export const isLang = (v: unknown): v is Lang => v === "en" || v === "pl";

const STORE_KEY = "oczi.lang";

/** Every page of the app reads the choice from here, so the overlay matches. */
export function setLang(next: Lang) {
  lang = isLang(next) ? next : "en";
  try {
    localStorage.setItem(STORE_KEY, lang);
  } catch {
    /* private mode or a page without storage — English stays */
  }
}

export function storedLang(): Lang {
  try {
    const v = localStorage.getItem(STORE_KEY);
    return isLang(v) ? v : "en";
  } catch {
    return "en";
  }
}

export const currentLang = () => lang;

/** The text for `key`, with `{}` placeholders filled in order. */
export function t(key: TextKey, ...args: (string | number)[]): string {
  const raw = TABLES[lang][key] ?? TABLES.en[key] ?? key;
  let i = 0;
  return raw.replace(/\{\}/g, () => String(args[i++] ?? ""));
}

export const table = (l: Lang) => TABLES[l];
export const KEY_COUNT = Object.keys(EN).length;
