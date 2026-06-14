<script lang="ts">
  import { t } from '$lib/i18n';

  type Mode = 'add' | 'rename' | 'recolor';

  let {
    open,
    mode,
    current = '',
    currentColor = 'White',
    onSubmit,
    onCancel
  }: {
    open: boolean;
    mode: Mode;
    current?: string;
    currentColor?: string;
    onSubmit: (v: { name: string; color: string; description: string }) => void;
    onCancel: () => void;
  } = $props();

  const COLORS = [
    'Cyan',
    'Green',
    'Yellow',
    'Magenta',
    'Blue',
    'Red',
    'White',
    'Gray',
    'DarkCyan',
    'DarkGreen',
    'DarkYellow',
    'DarkMagenta',
    'DarkBlue',
    'DarkRed'
  ];
  const SWATCH: Record<string, string> = {
    Cyan: '#22d3ee',
    Green: '#34d399',
    Yellow: '#fbbf24',
    Magenta: '#e879f9',
    Blue: '#60a5fa',
    Red: '#f87171',
    White: '#e5e7eb',
    Gray: '#9ca3af',
    DarkCyan: '#0e7490',
    DarkGreen: '#15803d',
    DarkYellow: '#a16207',
    DarkMagenta: '#a21caf',
    DarkBlue: '#1d4ed8',
    DarkRed: '#b91c1c'
  };

  let name = $state('');
  let color = $state('White');
  let description = $state('');
  let seeded = ''; // plain guard: re-seed each time the dialog opens

  $effect(() => {
    const key = `${open}:${mode}:${current}`;
    if (open && key !== seeded) {
      name = mode === 'rename' ? current : '';
      color = mode === 'recolor' ? currentColor : 'White';
      description = '';
      seeded = key;
    }
  });

  const title = $derived(
    mode === 'add'
      ? t('profiles.dlgAddTitle')
      : mode === 'rename'
        ? t('profiles.dlgRenameTitle', { name: current })
        : t('profiles.dlgRecolorTitle', { name: current })
  );
  const nameValid = $derived(/^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/.test(name));
  const canSubmit = $derived(mode === 'recolor' ? true : nameValid);

  function submit() {
    if (!canSubmit) return;
    onSubmit({ name: name.trim(), color, description: description.trim() });
  }
</script>

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && onCancel()} />

{#if open}
  <div class="overlay">
    <button type="button" class="backdrop" aria-label={t('profiles.dlgClose')} onclick={onCancel}></button>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <h3>{title}</h3>

      {#if mode === 'add' || mode === 'rename'}
        <label class="fld">
          <span>{mode === 'rename' ? t('profiles.dlgNewName') : t('profiles.dlgName')}</span>
          <input
            class="sw-input"
            bind:value={name}
            placeholder={t('profiles.dlgNamePlaceholder')}
            title={t('profiles.dlgNameTip')}
            spellcheck="false"
            autocomplete="off"
          />
          {#if name && !nameValid}
            <span class="err">{t('profiles.dlgNameError')}</span>
          {/if}
        </label>
      {/if}

      {#if mode === 'add' || mode === 'recolor'}
        <label class="fld">
          <span>{t('profiles.dlgColor')}</span>
          <div class="colors">
            {#each COLORS as c (c)}
              <button
                type="button"
                class="swatch"
                class:sel={color === c}
                style="background:{SWATCH[c]}"
                title={c}
                aria-label={c}
                onclick={() => (color = c)}
              ></button>
            {/each}
          </div>
        </label>
      {/if}

      {#if mode === 'add'}
        <label class="fld">
          <span>{t('profiles.dlgDescription')}</span>
          <input class="sw-input" bind:value={description} placeholder={t('profiles.dlgDescriptionPlaceholder')} title={t('profiles.dlgDescriptionTip')} spellcheck="false" />
        </label>
      {/if}

      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('profiles.dlgCancelTip')}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit} title={t('profiles.dlgSubmitTip')}>
          {mode === 'add' ? t('profiles.dlgAdd') : mode === 'rename' ? t('profiles.dlgRename') : t('common.apply')}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
  }
  .backdrop {
    position: absolute;
    inset: 0;
    border: none;
    padding: 0;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(2px);
    cursor: default;
  }
  .dialog {
    position: relative;
    width: min(440px, 92vw);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg);
    padding: var(--sw-space-6);
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
  }
  h3 {
    margin: 0 0 var(--sw-space-4);
    font-size: 1rem;
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .fld {
    display: block;
    margin-bottom: var(--sw-space-4);
  }
  .fld > span {
    display: block;
    margin-bottom: 6px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .err {
    margin-top: 4px;
    color: #f59e0b;
    font-size: var(--sw-text-xs);
  }
  .colors {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .swatch {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
  }
  .swatch.sel {
    border-color: var(--sw-text-primary);
    box-shadow: 0 0 0 2px var(--sw-bg-secondary);
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-6);
  }
</style>
