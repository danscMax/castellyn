export type Theme = 'dark' | 'light';
const KEY = 'cmh-theme';

export function getTheme(): Theme {
  if (typeof localStorage === 'undefined') return 'dark';
  return (localStorage.getItem(KEY) as Theme) ?? 'dark';
}

export function applyTheme(t: Theme): void {
  document.documentElement.classList.toggle('light', t === 'light');
  if (typeof localStorage !== 'undefined') localStorage.setItem(KEY, t);
}

export function initTheme(): void {
  applyTheme(getTheme());
}
