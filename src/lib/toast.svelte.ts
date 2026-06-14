// Tiny app-wide toast store (Svelte 5 runes). Surfaces operation outcomes so users get a
// glanceable result without reading the execution log. Errors are sticky (manual dismiss).
export type ToastKind = 'success' | 'warn' | 'error' | 'info';
export type ToastAction = { label: string; onClick: () => void };
export type Toast = {
  id: number;
  kind: ToastKind;
  title: string;
  detail?: string;
  action?: ToastAction;
};

let seq = 0;
export const toastStore = $state<{ items: Toast[] }>({ items: [] });

export function pushToast(t: Omit<Toast, 'id'>, ttlMs = 6000): number {
  const id = ++seq;
  toastStore.items.push({ ...t, id });
  if (t.kind !== 'error' && ttlMs > 0) {
    setTimeout(() => dismiss(id), ttlMs);
  }
  return id;
}

export function dismiss(id: number): void {
  toastStore.items = toastStore.items.filter((x) => x.id !== id);
}
