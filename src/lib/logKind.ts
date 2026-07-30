// Classify a console log line for coloring. The failure vocabulary is ru+en (V11: was
// English-only, so a Russian "Ошибка:" / "не удалось" never read as an error). Pure + unit-tested;
// Console.svelte classifies each visible line once (P7) instead of running regexes in the template.

export type LineKind = '' | 'warn' | 'diag' | 'ok' | 'err';

// Tools that emit their own severity get believed over the keyword scan below. cargo-binstall logs
// `WARN Attempting at atomic rename failed: Отказано в доступе …, fallback to other methods.` and
// then succeeds — the word "failed" describes an attempt it recovered from, not the outcome. Painting
// that red made a clean install look broken, which is the same class of lie as a green run that did
// nothing. An explicit ERROR prefix still wins; only the tool's own WARN/INFO/DEBUG are trusted.
const LEVEL = /^\s*(?:\[)?(ERROR|ERR|FATAL|WARN(?:ING)?|INFO|DEBUG|TRACE|NOTE)\b\]?[:\s]/i;

export function classifyLine(line: string): LineKind {
  if (line.startsWith('[diag]')) return 'diag';
  if (line.startsWith('✓')) return 'ok';
  const level = line.match(LEVEL)?.[1]?.toUpperCase();
  if (level) {
    if (level === 'ERROR' || level === 'ERR' || level === 'FATAL') return 'err';
    return level.startsWith('WARN') ? 'warn' : '';
  }
  // err wins over the ⚠ warn prefix (matches the prior CSS source-order precedence).
  if (/error|fail|exception|ошибк|не удалось|сбой|отказ|провал/i.test(line)) return 'err';
  if (line.startsWith('⚠')) return 'warn';
  return '';
}
