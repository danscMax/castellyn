import { describe, it, expect } from 'vitest';
import { askPreview, cleanLine } from './askPreview';

describe('cleanLine', () => {
  it('drops box art and collapses whitespace', () => {
    expect(cleanLine('│  Do you want to proceed?      │')).toBe('Do you want to proceed?');
    expect(cleanLine('╭──────────────────────────────╮')).toBe('');
  });
});

describe('askPreview', () => {
  it('picks the question out of a prompt box, not its option rows', () => {
    const box = [
      '╭────────────────────────────────────────────╮',
      '│ Do you want to make this edit to lib.rs?   │',
      '│ ❯ 1. Yes                                   │',
      '│   2. No                                    │',
      '╰────────────────────────────────────────────╯',
      ''
    ];
    expect(askPreview(box)).toBe('Do you want to make this edit to lib.rs?');
  });

  it('ignores a long option row so the question still wins', () => {
    // The guard that matters: option rows are what a naive "longest line" would latch onto.
    expect(
      askPreview(['Run this command?', '  1. Yes, and never ask about this command again please'])
    ).toBe('Run this command?');
  });

  it('truncates with an ellipsis and returns empty when there is no prose', () => {
    expect(askPreview(['x'.repeat(200)], 20)).toBe('x'.repeat(19) + '…');
    expect(askPreview(['│', '   ', '> 1. Yes', 'ok'])).toBe('');
    expect(askPreview([])).toBe('');
  });
});
