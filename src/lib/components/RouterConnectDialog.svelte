<script lang="ts">
  import type { EngineStatus } from '$lib/ipc';
  import { readEngineModels } from '$lib/ipc';
  import { t } from '$lib/i18n';

  let {
    open,
    engine,
    profiles = [],
    onSubmit,
    onCancel
  }: {
    open: boolean;
    engine: EngineStatus | null;
    profiles?: string[];
    onSubmit: (v: { model: string; profile: string }) => void;
    onCancel: () => void;
  } = $props();

  let model = $state('');
  let profile = $state('');
  let models = $state<string[]>([]);
  let loading = $state(false);
  let seeded = '';

  $effect(() => {
    const key = `${open}:${engine?.id ?? ''}`;
    if (open && engine && key !== seeded) {
      seeded = key;
      model = '';
      profile = profiles[0] ?? '';
      models = [];
      // Auto-load models from the engine.
      loading = true;
      readEngineModels(engine.baseUrl)
        .then((m) => {
          models = m;
          if (m.length && !model) model = m[0];
        })
        .catch(() => (models = []))
        .finally(() => (loading = false));
    }
  });

  const canSubmit = $derived(!!model.trim() && !!profile);
  // Anthropic-native engines (LM Studio, GLM router) bind straight to the profile — no ccr.
  const direct = $derived(!!engine && engine.protocol === 'anthropic' && !engine.router);
</script>

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && onCancel()} />

{#if open && engine}
  <div class="overlay">
    <button type="button" class="backdrop" aria-label={t('providers.dialogClose')} onclick={onCancel}></button>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <h3>{direct ? t('providers.rcBindTitle', { name: engine.name }) : t('providers.rcConnectTitle', { name: engine.name })}</h3>
      <p class="sub">
        {#if direct}
          {t('providers.rcDirectSub', { url: engine.baseUrl })}
        {:else}
          {t('providers.rcRouterSub', { url: engine.baseUrl })}
        {/if}
      </p>

      <label class="fld">
        <span>{loading ? t('providers.rcModelLoading') : models.length ? t('providers.rcModelAvailable', { n: models.length }) : t('providers.rcModelManual')}</span>
        <input class="sw-input" list="rc-models" bind:value={model} placeholder={t('providers.rcModelPlaceholder')} spellcheck="false" title={t('providers.rcModelInputTip')} />
        <datalist id="rc-models">
          {#each models as m (m)}<option value={m}></option>{/each}
        </datalist>
      </label>

      <label class="fld">
        <span>{t('providers.rcProfileLabel')}</span>
        <select class="sw-input" bind:value={profile} title={t('providers.rcProfileSelectTip')}>
          {#each profiles as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      </label>

      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('providers.dialogCancelTip')}>{t('providers.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={() => onSubmit({ model: model.trim(), profile })}
          title={direct ? t('providers.rcBindTip') : t('providers.rcConnectTip')}>
          {direct ? t('providers.rcBind') : t('providers.rcConnect')}
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
    width: min(460px, 94vw);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-lg);
    padding: var(--sw-space-6);
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
  }
  h3 {
    margin: 0 0 var(--sw-space-2);
    font-size: 1rem;
    font-weight: 600;
    color: var(--sw-text-primary);
  }
  .sub {
    margin: 0 0 var(--sw-space-4);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    line-height: 1.5;
  }
  .fld {
    display: block;
    margin-bottom: var(--sw-space-3);
  }
  .fld > span {
    display: block;
    margin-bottom: 6px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-6);
  }
</style>
