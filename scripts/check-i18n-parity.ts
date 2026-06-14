/**
 * i18n parity checker — verifies that ru.ts, en.ts and zh.ts expose exactly the
 * same set of leaf key paths (e.g. "settings.theme", "page.confirm_apply_msg").
 *
 * Run via: npm run check:i18n   (requires tsx)
 * Exit 0 = perfect parity; exit 1 = mismatches found.
 */

import ru from '../src/lib/i18n/locales/ru';
import en from '../src/lib/i18n/locales/en';
import zh from '../src/lib/i18n/locales/zh';

function getLeafKeys(obj: unknown, prefix = ''): string[] {
  if (typeof obj !== 'object' || obj === null) return [prefix];
  return Object.entries(obj as Record<string, unknown>).flatMap(([k, v]) =>
    getLeafKeys(v, prefix ? `${prefix}.${k}` : k)
  );
}

const dicts: Record<string, Set<string>> = {
  ru: new Set(getLeafKeys(ru)),
  en: new Set(getLeafKeys(en)),
  zh: new Set(getLeafKeys(zh))
};

// Reference = union of all keys; report any locale missing any key.
const all = new Set<string>([...dicts.ru, ...dicts.en, ...dicts.zh]);
let bad = false;

for (const [loc, keys] of Object.entries(dicts)) {
  const missing = [...all].filter((k) => !keys.has(k));
  if (missing.length) {
    bad = true;
    console.error(`\n[${loc}] missing ${missing.length} key(s):`);
    missing.forEach((k) => console.error(`  - ${k}`));
  }
}

if (bad) {
  process.exit(1);
}

console.log(`i18n parity OK — ${all.size} keys across ru / en / zh`);
