<script lang="ts">
  import { t } from '$lib/i18n';
  import { PROFILE_COLORS, PROFILE_SWATCH } from '$lib/statusColor';
  import ModalShell from './ModalShell.svelte';

  type Mode = 'add' | 'rename' | 'recolor' | 'redescribe';

  let {
    open,
    mode,
    current = '',
    currentColor = 'White',
    currentDescription = '',
    onSubmit,
    onCancel
  }: {
    open: boolean;
    mode: Mode;
    current?: string;
    currentColor?: string;
    currentDescription?: string;
    onSubmit: (v: { name: string; color: string; description: string }) => void;
    onCancel: () => void;
  } = $props();

  let name = $state('');
  let color = $state('White');
  let description = $state('');
  let seeded = ''; // plain guard: re-seed each time the dialog opens

  $effect(() => {
    if (!open) {
      seeded = ''; // reset on close so reopening the SAME item after Cancel reseeds from props
      return;
    }
    const key = `${open}:${mode}:${current}`;
    if (key !== seeded) {
      name = mode === 'rename' ? current : '';
      color = mode === 'recolor' ? currentColor : 'White';
      description = mode === 'redescribe' ? currentDescription : '';
      seeded = key;
    }
  });

  const title = $derived(
    mode === 'add'
      ? t('profiles.dlgAddTitle')
      : mode === 'rename'
        ? t('profiles.dlgRenameTitle', { name: current })
        : mode === 'redescribe'
          ? t('profiles.dlgRedescribeTitle', { name: current })
          : t('profiles.dlgRecolorTitle', { name: current })
  );
  const nameValid = $derived(/^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/.test(name));
  // recolor/redescribe don't touch the name, so they don't need a valid name to submit.
  const canSubmit = $derived(mode === 'recolor' || mode === 'redescribe' ? true : nameValid);

  function submit() {
    if (!canSubmit) return;
    onSubmit({ name: name.trim(), color, description: description.trim() });
  }
</script>

<ModalShell {open} onClose={onCancel} onEnter={submit} size="sm">
      <h3 class="dlg-h">{title}</h3>

      {#if mode === 'add' || mode === 'rename'}
        <label class="dlg-fld">
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
            <span class="err status-warn">{t('profiles.dlgNameError')}</span>
          {/if}
        </label>
      {/if}

      {#if mode === 'add' || mode === 'recolor'}
        <label class="dlg-fld">
          <span>{t('profiles.dlgColor')}</span>
          <div class="colors">
            {#each PROFILE_COLORS as c (c)}
              <button
                type="button"
                class="swatch"
                class:sel={color === c}
                style="background:{PROFILE_SWATCH[c]}"
                title={c}
                aria-label={c}
                onclick={() => (color = c)}
              ></button>
            {/each}
          </div>
        </label>
      {/if}

      {#if mode === 'add' || mode === 'redescribe'}
        <label class="dlg-fld">
          <span>{t('profiles.dlgDescription')}</span>
          <input class="sw-input" bind:value={description} placeholder={t('profiles.dlgDescriptionPlaceholder')} title={t('profiles.dlgDescriptionTip')} spellcheck="false" />
        </label>
      {/if}

      <div class="dlg-row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('profiles.dlgCancelTip')}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit} title={t('profiles.dlgSubmitTip')}>
          {mode === 'add' ? t('profiles.dlgAdd') : mode === 'rename' ? t('profiles.dlgRename') : t('common.apply')}
        </button>
      </div>
</ModalShell>

<style>
  /* This dialog spaces its fields a touch wider than the shared default. */
  .dlg-fld {
    margin-bottom: var(--sw-space-4);
  }
  .err {
    margin-top: 4px;
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
</style>
