// Print freellmapi's unified API key as {"key":"..."} — read-only over the gateway's
// own better-sqlite3 (same resolve pattern as query.cjs, located relative to the DB path).
// On any failure it prints {"error":"..."} and exits 0 so the caller treats it as "not set".
//
// Castellyn spawns this with: node unified-key.cjs <dbPath>
// The key goes to stdout only — never logged or persisted by the caller beyond `setx`.

'use strict';
const path = require('path');

function out(obj) { process.stdout.write(JSON.stringify(obj)); }

try {
  const dbPath = process.argv[2];
  if (!dbPath) { out({ error: 'usage: unified-key.cjs <dbPath>' }); process.exit(0); }

  const serverDir = path.dirname(path.dirname(dbPath)); // <server>/data/freeapi.db -> <server>
  const rootDir = path.dirname(serverDir);

  let Database = null;
  for (const cand of [
    path.join(serverDir, 'node_modules', 'better-sqlite3'),
    path.join(rootDir, 'node_modules', 'better-sqlite3'),
  ]) {
    try { Database = require(cand); break; } catch { /* try next */ }
  }
  if (!Database) { out({ error: 'better-sqlite3 not found in gateway install' }); process.exit(0); }

  const db = new Database(dbPath, { readonly: true, fileMustExist: true });
  const row = db.prepare("SELECT value FROM settings WHERE key = 'unified_api_key'").get();
  out(row && row.value ? { key: row.value } : { error: 'unified_api_key not found' });
} catch (e) {
  out({ error: String((e && e.message) || e) });
}
