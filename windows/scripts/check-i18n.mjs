
import { build } from "esbuild";
import { readFileSync, readdirSync, unlinkSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const SOURCE = join(HERE, "..", "src", "core", "i18n.ts");
const BUNDLE = join(HERE, ".i18n.bundle.mjs");

await build({ entryPoints: [SOURCE], bundle: true, format: "esm", outfile: BUNDLE, logLevel: "warning" });
const { table, LANGUAGES } = await import(pathToFileURL(BUNDLE).href);
unlinkSync(BUNDLE);

const placeholders = (s) => (s.match(/\{\}/g) || []).length;
const en = table("en");
const problems = [];

for (const { tag } of LANGUAGES) {
  if (tag === "en") continue;
  const other = table(tag);
  const keys = new Set([...Object.keys(en), ...Object.keys(other)]);
  for (const key of keys) {
    if (!(key in en)) problems.push(`${tag}: unknown key "${key}"`);
    else if (!(key in other)) problems.push(`${tag}: missing "${key}"`);
    else if (placeholders(other[key]) !== placeholders(en[key])) {
      problems.push(`${tag}: "${key}" has ${placeholders(other[key])} placeholders, English has ${placeholders(en[key])}`);
    }
  }
}

for (const [key, value] of Object.entries(en)) {
  if (!value.trim()) problems.push(`en: "${key}" is empty`);
}

// Any Polish left in a UI file means a string never went through t(), the way
// half the island once stayed Polish while English was the default.
const POLISH = new RegExp(
  "[ąćęłńóśźżĄĆĘŁŃÓŚŹŻ]|\\b(brak|otw[óo]rz|zobacz|kliknij|zapisz|usu[ńn]|wy[śs]lij|poka[żz]|gotowe|" +
    "b[łl][ąa]d|anuluj|zamknij|dzisiaj|teraz|minut|godzin|sekund|spotka[ńn]|modu[łl]|wersj|strona|stron|" +
    "lista|list[ęe]|dane|klucz|has[łl]o|ustawien|powiadom|zmian|pogod|od[śs]wie[żz]|pon[óo]w|zako[ńn]cz|" +
    "oczekuje|trwa|zaraz|jeszcze|tylko|razem|suma|wybierz|wpisz|wype[łl]nij|w[łl][ąa]cz|wy[łl][ąa]cz|" +
    "mo[żz]esz|musisz|nale[żz]y|dost[ęe]p|zapisano|usuni[ęe]to|u[żz]yw|zaplanowan|ostatni|nast[ęe]pn|" +
    "poprzedn|przez|je[żz]eli|kt[óo]r|si[ęe]|powinien|zamiast|prosz[ęe]|u[żz]ytkownik|zapytaj|" +
    "odpowiedz|plik|dzia[łl]a|wracam|chwil[ęe]|spos[óo]b|razie|potrzeb|wymaga|wyspa|zrzut|obszar|ekran|" +
    "przed chwil|wdro[żz]|p[łl]atno[śs][ćc]|przep[łl]yw|gwiazdek|tytu[łl]u|szczeg[óo][łl]y|wczytywanie)\\b",
  "i",
);

function uiFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...uiFiles(path));
    else if (entry.name.endsWith(".ts") && entry.name !== "i18n.ts") out.push(path);
  }
  return out;
}

function rustFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...rustFiles(path));
    else if (entry.name.endsWith(".rs")) out.push(path);
  }
  return out;
}

const ROOT = join(HERE, "..");
// The terminal tests quote a reply the model really sent, Polish and full-width
// pipes included, that text has to stay exactly as it was.
const RUST_SKIP = new Set(["text_tools.rs"]);

function scan(files) {
  for (const file of files) {
    if (RUST_SKIP.has(file.split(/[\\/]/).pop())) continue;
    const lines = readFileSync(file, "utf8").split("\n");
    lines.forEach((line, i) => {
      const code = line.trim();
      if (code.startsWith("//") || code.startsWith("*") || code.startsWith("/*")) return;
      // The HTML entity table maps names to letters like ó, which is not a word.
      if (/^"[a-z]+"\s*=>/.test(code)) return;
      if (POLISH.test(code)) problems.push(`${file.replace(ROOT, "").replace(/\\/g, "/")}:${i + 1} looks Polish: ${code.slice(0, 70)}`);
    });
  }
}

scan([
  ...uiFiles(join(ROOT, "src")),
  join(ROOT, "index.html"),
  join(ROOT, "settings.html"),
  join(ROOT, "snip.html"),
  ...rustFiles(join(ROOT, "src-tauri", "src")),
]);

const count = Object.keys(en).length;
if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} problem(s) with the translations.`);
  process.exit(1);
}
console.log(`i18n ok — ${count} keys in ${LANGUAGES.length} languages`);
