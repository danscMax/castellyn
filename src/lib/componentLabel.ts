import { t, hasTranslation } from '$lib/i18n';

/**
 * Display name for a maintenance component.
 *
 * The manifest is read from disk at runtime (`manifest_text()`), so it can declare components this
 * build ships no translation for — a user's own component, or one added ahead of its locale strings.
 * Those keep the name the manifest gives them rather than rendering a raw `components.x.name` key.
 */
export function componentName(id: string, manifestName: string): string {
  const key = `components.${id}.name`;
  return hasTranslation(key) ? t(key) : manifestName;
}
