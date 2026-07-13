<script lang="ts">
  import type { MyProvider, MyProviderInput } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import DropdownMenu from './DropdownMenu.svelte';
  import ModalShell from './ModalShell.svelte';
  import SecretInput from './SecretInput.svelte';
  import { isValidHttpUrl } from '$lib/url';

  let {
    open,
    current,
    profiles = [],
    onSubmit,
    onCancel
  }: {
    open: boolean;
    current: MyProvider | null;
    profiles?: string[];
    onSubmit: (p: MyProviderInput, apiKey: string) => void;
    onCancel: () => void;
  } = $props();

  let name = $state('');
  let baseUrl = $state('');
  let protocol = $state<'anthropic' | 'openai'>('openai');
  let model = $state('');
  let smallModel = $state('');
  let connectVia = $state<'freellmapi' | 'direct'>('freellmapi');
  let targetProfile = $state('');
  let balanceUrl = $state('');
  let apiKey = $state('');
  let advanced = $state(false);
  let seeded = '';

  $effect(() => {
    if (!open) {
      seeded = ''; // reset on close so reopening the SAME item after Cancel reseeds from props
      return;
    }
    const key = `${open}:${current?.id ?? ''}`;
    if (key !== seeded) {
      name = current?.name ?? '';
      baseUrl = current?.baseUrl ?? '';
      protocol = current?.protocol ?? 'openai';
      model = current?.model ?? '';
      smallModel = current?.smallModel ?? '';
      connectVia = current?.connectVia ?? 'freellmapi';
      targetProfile = current?.targetProfile ?? '';
      balanceUrl = current?.balanceUrl ?? '';
      apiKey = '';
      advanced = !!current?.smallModel || current?.protocol === 'anthropic' || !!current?.balanceUrl;
      seeded = key;
    }
  });

  const directOpenaiBlocked = $derived(connectVia === 'direct' && protocol === 'openai');
  const needsProfile = $derived(connectVia === 'direct');
  // Mirror the backend `valid_provider_name` (src-tauri/src/lib.rs): after trim, non-empty,
  // <= 64 chars, and no control chars (Unicode Cc: U+0000-001F, U+007F-009F). Keeps the UI from
  // accepting a name the backend will then reject.
  // eslint-disable-next-line no-control-regex
  const CONTROL_CHARS = /[\u0000-\u001f\u007f-\u009f]/;
  const nameValid = $derived.by(() => {
    const n = name.trim();
    return n.length > 0 && n.length <= 64 && !CONTROL_CHARS.test(n);
  });
  const canSubmit = $derived(
    nameValid && isValidHttpUrl(baseUrl.trim()) && (!needsProfile || !!targetProfile)
  );
  const viaLabel = $derived(
    connectVia === 'freellmapi' ? t('myProviders.viaFreellmapi') : t('myProviders.viaDirect')
  );
  const protoLabel = $derived(protocol === 'openai' ? 'OpenAI' : 'Anthropic');

  function submit() {
    if (!canSubmit) return;
    onSubmit(
      {
        id: current?.id,
        name: name.trim(),
        baseUrl: baseUrl.trim(),
        protocol,
        model: model.trim(),
        smallModel: smallModel.trim(),
        connectVia,
        targetProfile: needsProfile ? targetProfile : '',
        balanceUrl: balanceUrl.trim()
      },
      apiKey.trim()
    );
  }
</script>

<!-- U11: onEnter like the sibling form dialogs (submit self-guards on canSubmit) -->
<ModalShell {open} onClose={onCancel} onEnter={submit} size="md">
      <h3 class="dlg-h">{current?.id ? t('myProviders.editTitle') : t('myProviders.addTitle')}</h3>

      <label class="dlg-fld">
        <span>{t('myProviders.name')}</span>
        <input class="sw-input" bind:value={name} placeholder="DeepSeek" autocomplete="off" />
        {#if name.trim() && !nameValid}
          <span class="dlg-warn">{t('myProviders.errInvalidName')}</span>
        {/if}
      </label>

      <label class="dlg-fld">
        <span>{t('myProviders.baseUrl')}</span>
        <input class="sw-input" bind:value={baseUrl} placeholder="https://api.deepseek.com/v1" spellcheck="false" autocomplete="off" />
        {#if baseUrl.trim() && !isValidHttpUrl(baseUrl.trim())}
          <span class="dlg-warn">{t('myProviders.errInvalidUrl')}</span>
        {/if}
      </label>

      <div class="dlg-fld">
        <span>{t('myProviders.connectVia')}</span>
        <div class="ddwrap">
          <DropdownMenu
            label={viaLabel}
            align="left"
            items={[
              { label: t('myProviders.viaFreellmapi'), onClick: () => (connectVia = 'freellmapi') },
              { label: t('myProviders.viaDirect'), onClick: () => (connectVia = 'direct') }
            ]}
          />
        </div>
        <span class="dlg-hint">{t('myProviders.connectViaHint')}</span>
      </div>

      {#if needsProfile}
        <div class="dlg-fld">
          <span>{t('myProviders.targetProfile')}</span>
          <div class="ddwrap">
            <DropdownMenu
              label={targetProfile || t('myProviders.targetProfilePlaceholder')}
              align="left"
              items={profiles.map((p) => ({ label: p, onClick: () => (targetProfile = p) }))}
            />
          </div>
        </div>
      {/if}

      {#if directOpenaiBlocked}
        <p class="dlg-warn">{t('myProviders.openaiNeedsRouter')}</p>
      {/if}

      <label class="dlg-fld">
        <span>{t('myProviders.model')}</span>
        <input class="sw-input" bind:value={model} placeholder="deepseek-chat" spellcheck="false" />
      </label>

      <label class="dlg-fld">
        <span>{t('myProviders.apiKey')}</span>
        <SecretInput bind:value={apiKey}
          placeholder={current?.hasKey ? t('myProviders.apiKeyKeep') : t('myProviders.apiKeyPlaceholder')} />
      </label>

      <!-- Advanced: protocol + small model — hidden by default (most users never need them) -->
      <button type="button" class="adv-toggle" onclick={() => (advanced = !advanced)}>
        <span class="caret" class:open={advanced}>▸</span> {t('myProviders.advanced')}
      </button>
      {#if advanced}
        <div class="adv">
          <div class="dlg-fld">
            <span>{t('myProviders.protocol')}</span>
            <div class="ddwrap">
              <DropdownMenu
                label={protoLabel}
                align="left"
                items={[
                  { label: 'OpenAI', onClick: () => (protocol = 'openai') },
                  { label: 'Anthropic', onClick: () => (protocol = 'anthropic') }
                ]}
              />
            </div>
            <span class="dlg-hint">{t('myProviders.protocolHint')}</span>
          </div>
          <label class="dlg-fld">
            <span>{t('myProviders.smallModel')}</span>
            <input class="sw-input" bind:value={smallModel} placeholder={t('myProviders.smallModelPlaceholder')} spellcheck="false" />
            <span class="dlg-hint">{t('myProviders.smallModelHint')}</span>
          </label>
          <label class="dlg-fld">
            <span>{t('myProviders.balanceUrl')}</span>
            <input class="sw-input" bind:value={balanceUrl} placeholder="https://api.deepseek.com/user/balance" spellcheck="false" autocomplete="off" />
            <span class="dlg-hint">{t('myProviders.balanceUrlHint')}</span>
          </label>
        </div>
      {/if}

      <div class="dlg-row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit}>{t('myProviders.save')}</button>
      </div>
</ModalShell>

<style>
  .ddwrap :global(.dd),
  .ddwrap :global(.dd > button) {
    width: 100%;
  }
  .ddwrap :global(.dd > button) {
    justify-content: space-between;
    display: flex;
  }
  .adv-toggle {
    background: none;
    border: none;
    padding: 4px 0;
    margin: 2px 0 var(--sw-space-2);
    color: var(--sw-text-secondary);
    font-size: var(--sw-text-xs);
    cursor: pointer;
  }
  .adv-toggle .caret {
    display: inline-block;
    transition: transform 0.15s;
  }
  .adv-toggle .caret.open {
    transform: rotate(90deg);
  }
  .adv {
    border-left: 2px solid var(--sw-border);
    padding-left: var(--sw-space-3);
    margin-bottom: var(--sw-space-2);
  }
</style>
