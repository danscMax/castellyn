import { describe, it, expect } from 'vitest';
import { redactSecrets } from './redact';

describe('redactSecrets (L17)', () => {
  it('masks secret-ish JSON values but keeps structure and non-secrets', () => {
    const input = JSON.stringify(
      {
        model: 'claude-opus',
        env: { ANTHROPIC_AUTH_TOKEN: 'sk-ant-supersecret1234', PATH: '/usr/local/bin' },
        apiKey: 'abcdef1234567890xyz',
      },
      null,
      2,
    );
    const out = redactSecrets(input);
    expect(out).not.toContain('sk-ant-supersecret1234');
    expect(out).not.toContain('abcdef1234567890xyz');
    expect(out).toContain('claude-opus'); // non-secret value preserved
    expect(out).toContain('/usr/local/bin'); // PATH is not a secret key
    expect(out).toContain('[redacted]');
    // still valid-looking JSON structure (keys intact)
    expect(out).toContain('"ANTHROPIC_AUTH_TOKEN"');
    expect(out).toContain('"apiKey"');
  });

  it('masks standalone token shapes in prose (CLAUDE.md)', () => {
    const md = 'Use `sk-proj-abcdefghijkl12345` or set `Authorization: Bearer eyJhbGciOiJ12345abc`.';
    const out = redactSecrets(md);
    expect(out).not.toContain('sk-proj-abcdefghijkl12345');
    expect(out).not.toContain('eyJhbGciOiJ12345abc');
  });

  it('is a no-op on empty / secret-free text', () => {
    expect(redactSecrets('')).toBe('');
    const plain = '# Notes\njust plain text, "model": "claude", no secrets here';
    expect(redactSecrets(plain)).toBe(plain);
  });
});
