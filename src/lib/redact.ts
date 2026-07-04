// L17: redact secret-shaped values before showing a profile's raw settings.json / CLAUDE.md in the
// read-only config viewer (and before Copy). Claude Code's settings.json commonly embeds MCP-server
// `env` API keys in plaintext, and everywhere else in the UI secrets are masked — this viewer was the
// one place that dumped the file verbatim. Best-effort *textual* redaction so it works for both JSON
// and markdown; fail-secure (over-masking a rare false positive beats leaking a key on a screenshot).
const MASK = '••••••[redacted]';

// A JSON string value whose key name ends in a secret-ish word ("apiKey", "ANTHROPIC_AUTH_TOKEN", …).
const SECRET_KV =
  /("[^"]*(?:api[_-]?key|_?key|_?token|secret|password|passwd|auth[_-]?token|auth|credential|bearer)"\s*:\s*")([^"]+)"/gi;
// Provider/token prefixes (OpenAI sk-, xAI, GitHub PAT, GitLab, …).
const TOKEN_PREFIX = /\b(?:sk|xai|ghp|gho|ghs|pk|rk|glpat)-[A-Za-z0-9_-]{12,}\b/g;
// `Authorization: Bearer <token>` / bare `Bearer <token>`.
const BEARER = /(Bearer\s+)[A-Za-z0-9._~+/=-]{12,}/gi;

/** Mask secret-looking values in arbitrary config text (JSON or markdown). */
export function redactSecrets(text: string): string {
  if (!text) return text;
  return text
    .replace(SECRET_KV, (_m, keyPart: string) => `${keyPart}${MASK}"`)
    .replace(TOKEN_PREFIX, MASK)
    .replace(BEARER, `$1${MASK}`);
}
