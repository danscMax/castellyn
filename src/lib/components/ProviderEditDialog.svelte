<script lang="ts">
  import type { EngineStatus, ProfileProvider } from '$lib/ipc';
  import { readEngineModels } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import Toggle from './Toggle.svelte';

  let {
    open,
    profileName,
    current,
    engines = [],
    onSubmit,
    onCancel
  }: {
    open: boolean;
    profileName: string;
    current: ProfileProvider | null;
    engines?: EngineStatus[];
    onSubmit: (v: {
      baseUrl: string;
      token: string;
      model: string;
      smallModel: string;
      keepToken: boolean;
    }) => void;
    onCancel: () => void;
  } = $props();

  let baseUrl = $state('');
  let token = $state('');
  let model = $state('');
  let smallModel = $state('');
  let keepToken = $state(false);
  let seeded = '';

  $effect(() => {
    const key = `${open}:${profileName}:${current?.baseUrl ?? ''}`;
    if (open && key !== seeded) {
      baseUrl = current?.baseUrl ?? '';
      model = current?.model ?? '';
      smallModel = current?.smallModel ?? '';
      token = '';
      keepToken = !!current?.hasToken;
      seeded = key;
    }
  });

  // Warn when the chosen base URL matches an OpenAI-only engine (Claude Code needs a router).
  const matchedOpenAI = $derived(
    engines.find((e) => e.baseUrl && e.baseUrl === baseUrl && e.protocol === 'openai')
  );

  // Picking a preset: LM Studio's native Anthropic endpoint needs any non-empty bearer —
  // default it to 'lmstudio' (the documented token) so the profile binds without extra typing.
  function onPresetChange() {
    const eng = engines.find((e) => e.baseUrl === baseUrl);
    if (eng?.id === 'lmstudio' && !keepToken && !token.trim()) token = 'lmstudio';
  }
  const isCustomUrl = $derived(!!baseUrl && !engines.some((e) => e.baseUrl === baseUrl));
  const canSubmit = $derived(baseUrl.trim().length > 0);

  let models = $state<string[]>([]);
  let loadingModels = $state(false);
  async function loadModels() {
    if (!baseUrl.trim()) return;
    loadingModels = true;
    try {
      models = await readEngineModels(baseUrl.trim());
    } catch {
      models = [];
    }
    loadingModels = false;
  }

  function submit() {
    if (!canSubmit) return;
    onSubmit({
      baseUrl: baseUrl.trim(),
      token: token.trim(),
      model: model.trim(),
      smallModel: smallModel.trim(),
      keepToken: keepToken && !token.trim()
    });
  }
</script>

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && onCancel()} />

{#if open}
  <div class="overlay">
    <button type="button" class="backdrop" aria-label={t('providers.dialogClose')} onclick={onCancel}></button>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <h3>{t('providers.dialogTitle', { name: profileName })}</h3>

      <label class="fld">
        <span>{t('providers.presetLabel')}</span>
        <select class="sw-input" bind:value={baseUrl} onchange={onPresetChange} title={t('providers.presetSelectTip')}>
          <option value="" disabled>{t('providers.presetPlaceholder')}</option>
          {#each engines.filter((e) => e.protocol === 'anthropic') as e (e.id)}
            <option value={e.baseUrl}>{e.name} ({e.baseUrl})</option>
          {/each}
          {#if isCustomUrl}
            <option value={baseUrl}>{t('providers.presetCustom', { url: baseUrl })}</option>
          {/if}
        </select>
      </label>
      <p class="-mt-2 mb-sw-3 text-sw-xs text-sw-text-muted">
        {t('providers.presetHint')}
      </p>

      <label class="fld">
        <span>{t('providers.baseUrlLabel')}</span>
        <input class="sw-input" bind:value={baseUrl} placeholder="http://localhost:4000" spellcheck="false" autocomplete="off" title={t('providers.baseUrlInputTip')} />
        {#if matchedOpenAI}
          <span class="warn">{t('providers.openaiWarn')}</span>
        {/if}
      </label>

      <label class="fld">
        <span>{t('providers.tokenLabel')}</span>
        <input class="sw-input" type="password" bind:value={token} disabled={keepToken}
          placeholder={current?.hasToken ? t('providers.tokenSavedPlaceholder') : t('providers.tokenLocalPlaceholder')} autocomplete="off" title={t('providers.tokenInputTip')} />
        {#if current?.hasToken}
          <div class="chk">
            <Toggle bind:checked={keepToken} title={t('providers.keepTokenTitle')} />
            <span>{t('providers.keepToken')}</span>
          </div>
        {/if}
      </label>

      <div class="mb-sw-1 flex items-center justify-between">
        <span class="text-sw-xs text-sw-text-secondary">{t('providers.modelsLabel')}</span>
        <button type="button" class="sw-btn sw-btn-ghost text-sw-xs" disabled={!baseUrl.trim() || loadingModels}
          onclick={loadModels} title={t('providers.loadModelsTitle')}>
          {loadingModels ? t('providers.loading') : t('providers.loadModels')}
        </button>
      </div>
      {#if models.length}
        <p class="mb-sw-2 text-sw-xs text-sw-text-muted">{t('providers.modelsAvailable', { n: models.length })}</p>
      {/if}
      <datalist id="engine-models">
        {#each models as m (m)}<option value={m}></option>{/each}
      </datalist>
      <div class="two">
        <label class="fld">
          <span>{t('providers.modelLabel')}</span>
          <input class="sw-input" list="engine-models" bind:value={model} placeholder="glm-4.7" spellcheck="false" title={t('providers.modelInputTip')} />
        </label>
        <label class="fld">
          <span>{t('providers.smallModelLabel')}</span>
          <input class="sw-input" list="engine-models" bind:value={smallModel} placeholder="glm-4.5-air" spellcheck="false" title={t('providers.smallModelInputTip')} />
        </label>
      </div>

      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('providers.dialogCancelTip')}>{t('providers.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit} title={t('providers.applyProviderTip')}>{t('providers.apply')}</button>
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
    width: min(480px, 94vw);
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
    margin-bottom: var(--sw-space-3);
  }
  .fld > span {
    display: block;
    margin-bottom: 6px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--sw-space-3);
  }
  .warn {
    margin-top: 4px;
    color: #f59e0b;
    font-size: var(--sw-text-xs);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
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
