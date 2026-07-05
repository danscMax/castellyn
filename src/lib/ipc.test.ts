import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { sshTarget, STREAM_IDS, type SshHost } from './ipc';

// Cross-language parity: the TS STREAM_IDS mirror must match Rust `mod stream_id` exactly. We parse
// the const values straight out of lib.rs so a rename on either side breaks this test (no codegen).
describe('STREAM_IDS ↔ Rust stream_id parity', () => {
  it('matches the const set in lib.rs', () => {
    const libRs = fileURLToPath(new URL('../../src-tauri/src/lib.rs', import.meta.url));
    const src = readFileSync(libRs, 'utf8');
    const mod = src.match(/mod stream_id \{([\s\S]*?)\n\}/);
    expect(mod, 'mod stream_id block found in lib.rs').toBeTruthy();
    const rustValues = [...mod![1].matchAll(/pub const \w+: &str = "([^"]+)";/g)].map((m) => m[1]);
    expect(new Set(rustValues)).toEqual(new Set(Object.values(STREAM_IDS)));
  });
});

// Pins the SSH arg-injection hardening (a host re-tokenised by ssh.exe must be a single safe token,
// not just free of a leading '-'). Pure function — no tauri runtime touched.
describe('sshTarget arg-injection hardening', () => {
  const base: SshHost = { id: '1', name: 'x', host: '', source: 'saved' };

  it('builds a normal target', () => {
    expect(sshTarget({ ...base, host: 'example.com', user: 'me', port: 22 })).toBe('me@example.com -p 22');
  });

  it('smuggles nothing via whitespace in the host', () => {
    expect(() => sshTarget({ ...base, host: 'realhost -oProxyCommand=calc' })).toThrow();
  });

  it('rejects a leading-dash host', () => {
    expect(() => sshTarget({ ...base, host: '-oProxyCommand=calc' })).toThrow();
  });

  it('rejects a bad-charset / whitespace user', () => {
    expect(() => sshTarget({ ...base, host: 'h', user: 'a b' })).toThrow();
  });

  it('drops out-of-range ports instead of interpolating them', () => {
    expect(sshTarget({ ...base, host: 'h', port: 0 })).toBe('h');
    expect(sshTarget({ ...base, host: 'h', port: 70000 })).toBe('h');
  });

  it('quotes the key path and strips embedded quotes', () => {
    expect(sshTarget({ ...base, host: 'h', keyPath: 'C:\\my key"' })).toBe('h -i "C:\\my key"');
  });
});
