import { describe, it, expect, vi, beforeEach } from 'vitest';

// R7: saveConfig's optimistic-concurrency retry loop. Mock the Tauri invoke so we can drive
// read_config / write_config responses (including the 'config-conflict' sentinel).
const invokeMock = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invokeMock(...args) }));

const { saveConfig } = await import('./ipc');

describe('saveConfig (R7 optimistic concurrency)', () => {
  beforeEach(() => invokeMock.mockReset());

  it('reads, mutates, and writes back keyed on the base rev', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'read_config') return Promise.resolve({ rev: 3 });
      if (cmd === 'write_config') return Promise.resolve(4);
      return Promise.resolve();
    });
    await saveConfig((c) => (c.scriptsRoot = 'X'));
    const write = invokeMock.mock.calls.find((c) => c[0] === 'write_config');
    expect((write?.[1] as any).expectedRev).toBe(3);
    expect((write?.[1] as any).config.scriptsRoot).toBe('X');
  });

  it('retries after a config-conflict, then succeeds', async () => {
    let writes = 0;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'read_config') return Promise.resolve({ rev: writes });
      if (cmd === 'write_config') {
        writes++;
        return writes < 2 ? Promise.reject(new Error('config-conflict')) : Promise.resolve(writes);
      }
      return Promise.resolve();
    });
    await saveConfig((c) => (c.scriptsRoot = 'Y'));
    expect(writes).toBe(2); // one conflict, one success
  });

  it('gives up after 3 conflicting attempts', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'read_config') return Promise.resolve({ rev: 0 });
      if (cmd === 'write_config') return Promise.reject(new Error('config-conflict'));
      return Promise.resolve();
    });
    await expect(saveConfig((c) => (c.scriptsRoot = 'Z'))).rejects.toThrow('config-conflict');
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'write_config').length).toBe(3);
  });
});
