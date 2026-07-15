<script lang="ts">
  import type { EngineStatus } from '$lib/ipc';
  import { readEngineModels } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import Select from './Select.svelte';
  import ModalShell from './ModalShell.svelte';
  import SecretInput from './SecretInput.svelte';

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
    onSubmit: (v: { model: string; profile: string; key?: string }) => void;
    onCancel: () => void;
  } = $props();

  // Special target value: bind to the opencode agent instead of a Claude Code profile.
  const OPENCODE = '__opencode__';

  let model = $state('');
  let profile = $state('');
  let apiKey = $state('');
  let models = $state<string[]>([]);
  let loading = $state(false);
  let seeded = '';

  $effect(() => {
    if (!open) {
      seeded = ''; // reset on close so reopening the SAME engine after Cancel reseeds + refetches
      return;
    }
    const seed = engine?.id ?? '';
    if (engine && seed !== seeded) {
      seeded = seed;
      model = '';
      profile = profiles[0] ?? '';
      apiKey = '';
      models = [];
      // Auto-load models from the engine.
      loading = true;
      readEngineModels(engine.baseUrl)
        .then((m) => {
          if (seeded !== seed) return; // a newer open/engine superseded this request
          models = m;
          if (m.length && !model) model = m[0];
        })
        .catch(() => {
          if (seeded === seed) models = [];
        })
        .finally(() => {
          if (seeded === seed) loading = false;
        });
    }
  });

  const canSubmit = $derived(!!model.trim() && !!profile);
  // U11: Enter submits like in the sibling form dialogs (self-guards on validity).
  function submit() {
    if (!canSubmit) return;
    onSubmit({ model: model.trim(), profile, key: apiKey.trim() });
    apiKey = '';
  }
  // Anthropic-native engines (LM Studio, GLM router) bind straight to the profile — no ccr.
  const direct = $derived(!!engine && engine.protocol === 'anthropic' && !engine.router);
  // opencode is OpenAI-native → offered as a target only for openai-compatible engines.
  const allowOpencode = $derived(!!engine && engine.protocol === 'openai');
  const isOpencode = $derived(profile === OPENCODE);
  const profileOptions = $derived([
    ...(allowOpencode ? [{ value: OPENCODE, label: t('providers.rcOpencodeTarget') }] : []),
    ...profiles.map((p) => ({ value: p, label: p }))
  ]);
</script>

<ModalShell open={open && !!engine} onClose={onCancel} onEnter={submit} size="md">
    {#if engine}
      <h3 class="dlg-h sub-gap">
        {#if isOpencode}{t('providers.rcOpencodeTitle', { name: engine.name })}
        {:else if direct}{t('providers.rcBindTitle', { name: engine.name })}
        {:else}{t('providers.rcConnectTitle', { name: engine.name })}{/if}
      </h3>
      <p class="sub">
        {#if isOpencode}
          {t('providers.rcOpencodeSub', { url: engine.baseUrl })}
        {:else if direct}
          {t('providers.rcDirectSub', { url: engine.baseUrl })}
        {:else}
          {t('providers.rcRouterSub', { url: engine.baseUrl })}
        {/if}
      </p>

      <label class="dlg-fld">
        <span>{loading ? t('providers.rcModelLoading') : models.length ? t('providers.rcModelAvailable', { n: models.length }) : t('providers.rcModelManual')}</span>
        <input class="sw-input" list="rc-models" bind:value={model} placeholder={t('providers.rcModelPlaceholder')} spellcheck="false" title={t('providers.rcModelInputTip')} />
        <datalist id="rc-models">
          {#each models as m}<option value={m}></option>{/each}
        </datalist>
      </label>

      <div class="dlg-fld">
        <span>{t('providers.rcProfileLabel')}</span>
        <Select bind:value={profile} options={profileOptions} placeholder={t('providers.rcProfileLabel')} />
      </div>

      {#if isOpencode}
        <label class="dlg-fld">
          <span>{t('providers.rcOpencodeKeyLabel')}</span>
          <SecretInput bind:value={apiKey} placeholder={t('providers.rcOpencodeKeyPlaceholder')} title={t('providers.rcOpencodeKeyTip')} />
        </label>
      {/if}

      <div class="dlg-row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('providers.dialogCancelTip')}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit}
          title={isOpencode ? t('providers.rcOpencodeTip') : direct ? t('providers.rcBindTip') : t('providers.rcConnectTip')}>
          {isOpencode ? t('providers.rcOpencodeBtn') : direct ? t('providers.rcBind') : t('providers.rcConnect')}
        </button>
      </div>
    {/if}
</ModalShell>

<style>
  /* Tighter h3 margin than the shared .dlg-h: this title is immediately followed by .sub. */
  .sub-gap {
    margin-bottom: var(--sw-space-2);
  }
  .sub {
    margin: 0 0 var(--sw-space-4);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    line-height: 1.5;
  }
</style>
