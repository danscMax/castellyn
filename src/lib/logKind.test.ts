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

  it('plain lines are unclassified', () => {
    expect(classifyLine('Running the linter…')).toBe('');
    expect(classifyLine('3 files changed')).toBe('');
  });
});
