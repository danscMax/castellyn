// Validate a user-entered base URL: must parse AND use http/https (rejects ftp:, mailto:, foo:bar).
// Shared by the provider-edit dialogs so the accept rule can't drift between them — the frontend
// gate before the backend's own valid_base_url check.
export function isValidHttpUrl(s: string): boolean {
  try {
    const u = new URL(s);
    return u.protocol === 'http:' || u.protocol === 'https:';
  } catch {
    return false;
  }
}

// Host (host:port) of a URL, or the raw string if it doesn't parse — the shared "show just the host"
// idiom (was inlined as try { new URL(s).host } catch { s } across ProfilesTab / matrixState).
export function urlHost(s: string): string {
  try {
    return new URL(s).host;
  } catch {
    return s;
  }
}
