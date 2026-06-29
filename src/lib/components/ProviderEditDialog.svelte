<script lang="ts">
  import type { EngineStatus, ProfileProvider } from '$lib/ipc';
  import { readEngineModels } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import Toggle from './Toggle.svelte';
  import Select from './Select.svelte';
  import ModalShell from './ModalShell.svelte';
  import SecretInput from './SecretInput.svelte';
  import { isValidHttpUrl } from '$lib/url';

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
  const presetOptions = $derived([
    ...engines
      .filter((e) => e.protocol === 'anthropic' && e.baseUrl)
      .map((e) => ({ value: e.baseUrl, label: `${e.name} (${e.baseUrl})` })),
    ...(isCustomUrl ? [{ value: baseUrl, label: t('providers.presetCustom', { url: baseUrl }) }] : [])
  ]);
  // Block obviously malformed baseUrls from reaching the backend (shared strict http(s) check).
  const canSubmit = $derived(isValidHttpUrl(baseUrl.trim()));

  let models = $state<string[]>([]);
  let loadingModels = $state(false);
  let modelsMsg = $state(''); // feedback when the fetch returns nothing or errors (was silent)
  async function loadModels() {
    if (!baseUrl.trim()) return;
    loadingModels = true;
    modelsMsg = '';
    try {
      models = await readEngineModels(baseUrl.trim());
      if (!models.length) modelsMsg = t('providers.modelsNone');
    } catch {
      models = [];
      modelsMsg = t('providers.modelsError');
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

<ModalShell {open} onClose={onCancel} onEnter={submit} size="md">
      <h3 class="dlg-h">{t('providers.dialogTitle', { name: profileName })}</h3>

      <div class="dlg-fld">
        <span>{t('providers.presetLabel')}</span>
        <Select bind:value={baseUrl} options={presetOptions} placeholder={t('providers.presetPlaceholder')} onChange={onPresetChange} />
      </div>
      <p class="-mt-2 mb-sw-3 text-sw-xs text-sw-text-muted">
        {t('providers.presetHint')}
      </p>

      <label class="dlg-fld">
        <span>{t('providers.baseUrlLabel')}</span>
        <input class="sw-input" bind:value={baseUrl} placeholder="http://localhost:4000" spellcheck="false" autocomplete="off" title={t('providers.baseUrlInputTip')} />
        {#if matchedOpenAI}
          <span class="warn">{t('providers.openaiWarn')}</span>
        {:else if baseUrl.trim() && !isValidHttpUrl(baseUrl.trim())}
          <span class="warn">{t('providers.invalidUrl')}</span>
        {/if}
      </label>

      <label class="dlg-fld">
        <span>{t('providers.tokenLabel')}</span>
        <SecretInput bind:value={token} disabled={keepToken}
          placeholder={current?.hasToken ? t('providers.tokenSavedPlaceholder') : t('providers.tokenLocalPlaceholder')} title={t('providers.tokenInputTip')} />
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
      {:else if modelsMsg}
        <p class="mb-sw-2 text-sw-xs text-sw-text-muted">{modelsMsg}</p>
      {/if}
      <datalist id="engine-models">
        {#each models as m (m)}<option value={m}></option>{/each}
      </datalist>
      <div class="two">
        <label class="dlg-fld">
          <span>{t('providers.modelLabel')}</span>
          <input class="sw-input" list="engine-models" bind:value={model} placeholder="glm-4.7" spellcheck="false" title={t('providers.modelInputTip')} />
        </label>
        <label class="dlg-fld">
          <span>{t('providers.smallModelLabel')}</span>
          <input class="sw-input" list="engine-models" bind:value={smallModel} placeholder="glm-4.5-air" spellcheck="false" title={t('providers.smallModelInputTip')} />
        </label>
      </div>

      <div class="dlg-row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('providers.dialogCancelTip')}>{t('providers.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit} title={t('providers.applyProviderTip')}>{t('providers.apply')}</button>
      </div>
</ModalShell>

<style>
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--sw-space-3);
  }
  /* Inline (not block) warning, shown right under the base-URL input. */
  .warn {
    margin-top: 4px;
    color: var(--sw-warn);
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
</style>
