// Read-only analytics query over freellmapi's SQLite DB (freeapi.db).
//
// AgentHub spawns this with: node query.js <dbPath> <rangeHours>
// It opens the DB read-only (WAL-safe via better-sqlite3) so it never disturbs the
// live gateway, runs the same aggregates as freellmapi's /api/analytics summary +
// by-model routes (server/dist/routes/analytics.js), and prints one JSON object.
// On any failure it prints {"error": "..."} and exits 0 so the UI shows "no data".
//
// better-sqlite3 and the price fallbacks are resolved from the gateway install,
// located relative to the DB path (dbPath = <serverDir>/data/freeapi.db).

'use strict';
const path = require('path');

function out(obj) { process.stdout.write(JSON.stringify(obj)); }

try {
  const dbPath = process.argv[2];
  const rangeHours = parseInt(process.argv[3], 10);
  if (!dbPath || !Number.isFinite(rangeHours) || rangeHours <= 0) {
    out({ error: 'usage: query.js <dbPath> <rangeHours>' });
    process.exit(0);
  }

  const serverDir = path.dirname(path.dirname(dbPath)); // <server>/data/freeapi.db -> <server>
  const rootDir = path.dirname(serverDir);              // <server> -> <freellmapi root>

  // Resolve better-sqlite3 from the gateway install (server/.. node_modules).
  let Database = null;
  for (const cand of [
    path.join(serverDir, 'node_modules', 'better-sqlite3'),
    path.join(rootDir, 'node_modules', 'better-sqlite3'),
  ]) {
    try { Database = require(cand); break; } catch { /* try next */ }
  }
  if (!Database) { out({ error: 'better-sqlite3 not found in gateway install' }); process.exit(0); }

  // Price fallbacks for unmapped models — mirror freellmapi so savings match the dashboard.
  let FIN = 0.20, FOUT = 0.80;
  try {
    const p = require(path.join(serverDir, 'dist', 'db', 'model-pricing.js'));
    if (Number.isFinite(p.FALLBACK_INPUT_PER_M)) FIN = p.FALLBACK_INPUT_PER_M;
    if (Number.isFinite(p.FALLBACK_OUTPUT_PER_M)) FOUT = p.FALLBACK_OUTPUT_PER_M;
  } catch { /* keep defaults */ }

  const db = new Database(dbPath, { readonly: true, fileMustExist: true });
  const since = `-${rangeHours} hours`;

  const s = db.prepare(`
    SELECT
      COUNT(*) AS total_requests,
      SUM(CASE WHEN r.status = 'success' THEN 1 ELSE 0 END) AS success_count,
      SUM(r.input_tokens)  AS total_input_tokens,
      SUM(r.output_tokens) AS total_output_tokens,
      AVG(r.latency_ms)    AS avg_latency_ms,
      MIN(r.created_at)    AS first_request_at,
      SUM(CASE WHEN r.status = 'success' THEN
        r.input_tokens  * COALESCE(m.paid_input_per_m,  ?) / 1000000.0 +
        r.output_tokens * COALESCE(m.paid_output_per_m, ?) / 1000000.0
      ELSE 0 END) AS est_savings
    FROM requests r
    LEFT JOIN models m ON m.platform = r.platform AND m.model_id = r.model_id
    WHERE r.created_at >= datetime('now', ?)
  `).get(FIN, FOUT, since);

  const total = s.total_requests ?? 0;
  const rows = db.prepare(`
    SELECT
      r.platform, r.model_id, m.display_name,
      COUNT(*) AS requests,
      SUM(CASE WHEN r.status = 'success' THEN 1 ELSE 0 END) * 100.0 / COUNT(*) AS success_rate,
      AVG(r.latency_ms) AS avg_latency_ms,
      SUM(r.input_tokens)  AS total_input_tokens,
      SUM(r.output_tokens) AS total_output_tokens,
      SUM(CASE WHEN r.status = 'success' THEN
        r.input_tokens  * COALESCE(m.paid_input_per_m,  ?) / 1000000.0 +
        r.output_tokens * COALESCE(m.paid_output_per_m, ?) / 1000000.0
      ELSE 0 END) AS est_cost
    FROM requests r
    LEFT JOIN models m ON m.platform = r.platform AND m.model_id = r.model_id
    WHERE r.created_at >= datetime('now', ?)
    GROUP BY r.platform, r.model_id
    ORDER BY requests DESC
  `).all(FIN, FOUT, since);
  db.close();

  out({
    totals: {
      totalRequests: total,
      successRate: total > 0 ? Math.round((s.success_count / total) * 1000) / 10 : 0,
      totalInputTokens: s.total_input_tokens ?? 0,
      totalOutputTokens: s.total_output_tokens ?? 0,
      avgLatencyMs: Math.round(s.avg_latency_ms ?? 0),
      estimatedCostSavings: Math.round((s.est_savings ?? 0) * 100) / 100,
      firstRequestAt: s.first_request_at ?? null,
    },
    perModel: rows.map((r) => ({
      platform: r.platform,
      modelId: r.model_id,
      displayName: r.display_name ?? r.model_id,
      requests: r.requests,
      successRate: Math.round((r.success_rate ?? 0) * 10) / 10,
      avgLatencyMs: Math.round(r.avg_latency_ms ?? 0),
      totalInputTokens: r.total_input_tokens ?? 0,
      totalOutputTokens: r.total_output_tokens ?? 0,
      estimatedCost: Math.round((r.est_cost ?? 0) * 100) / 100,
    })),
  });
} catch (e) {
  out({ error: String((e && e.message) || e) });
  process.exit(0);
}
