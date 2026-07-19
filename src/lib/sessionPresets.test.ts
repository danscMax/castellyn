import { describe, it, expect } from 'vitest';
import { ARG_PRESETS, isRiskyFlag, stripFlags, toggleFlag } from './sessionPresets';

// These two functions build the actual CLI args a session is launched with, including the
// skip-permissions / skip-approvals flags — a mangled result is a safety problem, not a cosmetic one.
describe('toggleFlag', () => {
  it('adds a flag that is absent and removes it again (idempotent round trip)', () => {
    const on = toggleFlag('', '--continue');
    expect(on).toBe('--continue');
    expect(toggleFlag(on, '--continue')).toBe('');
  });

  it('keeps the rest of the args when removing', () => {
    expect(toggleFlag('--continue --yolo', '--continue')).toBe('--yolo');
  });

  it('never yields both --effort levels at once', () => {
    const out = toggleFlag('--effort max', '--effort high');
    expect(out).toBe('--effort high');
    expect(out).not.toContain('max');
  });

  it('replaces --effort even with other args around it', () => {
    expect(toggleFlag('--continue --effort max --resume', '--effort high')).toBe(
      '--continue --resume --effort high'
    );
  });

  // Substring matching used to see '--search' inside '--search-all' and strip the prefix,
  // turning a valid arg into '-all'.
  it('does not match a flag that is only a PREFIX of a longer token', () => {
    expect(toggleFlag('--search-all', '--search')).toBe('--search-all --search');
    expect(toggleFlag('--search-all --search', '--search')).toBe('--search-all');
  });
});

describe('stripFlags', () => {
  it('leaves a custom remainder untouched', () => {
    expect(stripFlags('--continue --model opus --yolo', ARG_PRESETS.codex)).toBe('--continue --model opus');
  });

  it('removes every preset flag it finds', () => {
    expect(stripFlags('--yolo --full-auto --search', ARG_PRESETS.codex)).toBe('');
  });

  it('does not eat a longer token that merely starts with a preset flag', () => {
    expect(stripFlags('--search-all', ARG_PRESETS.codex)).toBe('--search-all');
  });

  it('collapses the whitespace left behind', () => {
    expect(stripFlags('a  --yolo   b', ARG_PRESETS.codex)).toBe('a b');
  });
});

describe('isRiskyFlag', () => {
  it('flags the approval/permission bypasses', () => {
    expect(isRiskyFlag('--dangerously-skip-permissions')).toBe(true);
    expect(isRiskyFlag(' --yolo ')).toBe(true);
    expect(isRiskyFlag('--full-auto')).toBe(true);
  });

  it('leaves harmless flags alone', () => {
    expect(isRiskyFlag('--continue')).toBe(false);
    expect(isRiskyFlag('--effort')).toBe(false);
  });
});
