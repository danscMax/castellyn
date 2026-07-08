// Classify a console log line for coloring. The failure vocabulary is ru+en (V11: was
// English-only, so a Russian "Ошибка:" / "не удалось" never read as an error). Pure + unit-tested;
// Console.svelte classifies each visible line once (P7) instead of running regexes in the template.

export type LineKind = '' | 'warn' | 'diag' | 'ok' | 'err';

export function classifyLine(line: string): LineKind {
  if (line.startsWith('[diag]')) return 'diag';
  if (line.startsWith('✓')) return 'ok';
  // err wins over the ⚠ warn prefix (matches the prior CSS source-order precedence).
  if (/error|fail|exception|ошибк|не удалось|сбой|отказ|провал/i.test(line)) return 'err';
  if (line.startsWith('⚠')) return 'warn';
  return '';
}
