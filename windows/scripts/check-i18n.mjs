
import { build } from "esbuild";
import { unlinkSync } from "node:fs";
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

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} problem(s) in the translations.`);
  process.exit(1);
}

const count = Object.keys(en).length;
console.log(`i18n ok — ${count} keys in ${LANGUAGES.length} languages`);
