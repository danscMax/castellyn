// Beginner-friendly explanations of each maintenance component (shown on its card).
// Keyed by the component id from the manifest. Text lives in the i18n `glossary`
// namespace; this helper resolves it reactively at read time (Svelte 5 runes),
// so the explanation re-renders on locale change.
import { t, hasTranslation } from '$lib/i18n';

// The component ids that have a glossary entry. Used to decide whether to render
// the explanation paragraph at all (no entry → no paragraph).
const GLOSSARY_IDS = [
  'all',
  'plugins',
  'forks',
  'rtk',
  'speckit',
  'opencode',
  'freellmapi',
  'cargo',
  'bomfix',
  'ccrrouter'
] as const;

/** Reactive glossary text for a component id, or '' when there is no entry. */
export function glossaryText(id: string): string {
  if (!(GLOSSARY_IDS as readonly string[]).includes(id)) return '';
  const key = `glossary.${id}`;
  return hasTranslation(key) ? t(key) : '';
}
