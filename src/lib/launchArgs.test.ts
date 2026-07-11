import { describe, it, expect } from 'vitest';
import { composeLaunchArgs } from './launchArgs';

describe('composeLaunchArgs', () => {
  it('claude: prepends structured effort and model exactly once', () => {
    expect(composeLaunchArgs('claude', '--verbose', { claudeEffort: 'high', claudeModel: 'opus' }))
      .toBe('--effort high --model opus --verbose');
  });

  it('claude: hand-typed flag wins — structured value is not doubled', () => {
    expect(composeLaunchArgs('claude', '--effort low', { claudeEffort: 'max' }))
      .toBe('--effort low');
    expect(composeLaunchArgs('claude', '-m sonnet', { claudeModel: 'opus', claudeEffort: 'medium' }))
      .toBe('--effort medium -m sonnet');
  });

  it('claude: empty structured selections preserve the advanced args untouched', () => {
    expect(composeLaunchArgs('claude', '--dangerously-skip-permissions', {}))
      .toBe('--dangerously-skip-permissions');
    expect(composeLaunchArgs('claude', '', {})).toBe('');
  });

  it('codex / opencode behaviour is unchanged (regression guard)', () => {
    expect(composeLaunchArgs('codex', '--yolo', { codexProfile: 'work', codexModel: 'gpt-5.4' }))
      .toBe('--profile work --model gpt-5.4 --yolo');
    expect(composeLaunchArgs('opencode', '', { opencodeModel: 'freellmapi/auto' }))
      .toBe('--model freellmapi/auto');
    // codex ignores claude fields
    expect(composeLaunchArgs('codex', '', { claudeEffort: 'high' })).toBe('');
  });

  it('shell / unknown env composes nothing', () => {
    expect(composeLaunchArgs('shell', '', { claudeEffort: 'high' })).toBe('');
  });
});
