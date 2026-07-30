import { describe, it, expect } from 'vitest';
import { classifyLine } from './logKind';

describe('classifyLine', () => {
  it('V11: classifies Russian failure words as err', () => {
    expect(classifyLine('Ошибка: не удалось запустить')).toBe('err');
    expect(classifyLine('Сбой синхронизации')).toBe('err');
    expect(classifyLine('Отказ доступа')).toBe('err');
    expect(classifyLine('Провал smoke-тестов')).toBe('err');
  });

  it('classifies English failure words as err', () => {
    expect(classifyLine('Error: connection refused')).toBe('err');
    expect(classifyLine('build FAILED')).toBe('err');
    expect(classifyLine('uncaught exception')).toBe('err');
  });

  it('prefix kinds: diag / ok / warn', () => {
    expect(classifyLine('[diag] probing port 13001')).toBe('diag');
    expect(classifyLine('✓ done')).toBe('ok');
    expect(classifyLine('⚠ pinned back')).toBe('warn');
  });

  it('err wins over the warn prefix', () => {
    expect(classifyLine('⚠ error while pinning')).toBe('err');
  });

  it("a tool's own severity prefix beats the keyword scan", () => {
    // Real lines from a cargo-binstall run that SUCCEEDED — each names a failed attempt and the
    // fallback it took. Reading them as errors made a clean install look broken.
    expect(
      classifyLine('WARN Attempting at atomic rename failed: Отказано в доступе. (os error 5), fallback to other methods.')
    ).toBe('warn');
    expect(classifyLine('WARN ReplaceFileW failed: Не удается удалить заменяемый файл.')).toBe('warn');
    expect(classifyLine('INFO Done in 9.4063042s')).toBe('');
    // …but a tool that says ERROR is still an error, and so is an unprefixed failure line.
    expect(classifyLine('ERROR could not write the manifest')).toBe('err');
    expect(classifyLine('[ERROR] build failed')).toBe('err');
    expect(classifyLine('cargo install failed')).toBe('err');
  });

  it('plain lines are unclassified', () => {
    expect(classifyLine('Running the linter…')).toBe('');
    expect(classifyLine('3 files changed')).toBe('');
  });
});
