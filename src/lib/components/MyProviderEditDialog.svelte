<script lang="ts">
  import type { MyProvider, MyProviderInput } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import DropdownMenu from './DropdownMenu.svelte';
  import ModalShell from './ModalShell.svelte';
  import SecretInput from './SecretInput.svelte';

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
    const key = `${open}:${current?.id ?? ''}`;
    if (open && key !== seeded) {
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

  function isValidUrl(s: string): boolean {
    try {
      const u = new URL(s);
      return u.protocol === 'http:' || u.protocol === 'https:';
    } catch {
      return false;
    }
  }
  const directOpenaiBlocked = $derived(connectVia === 'direct' && protocol === 'openai');
  const needsProfile = $derived(connectVia === 'direct');
  const canSubmit = $derived(
    !!name.trim() && isValidUrl(baseUrl.trim()) && (!needsProfile || !!targetProfile)
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

<ModalShell {open} onClose={onCancel} size="md">
      <h3>{current?.id ? t('myProviders.editTitle') : t('myProviders.addTitle')}</h3>

      <label class="fld">
        <span>{t('myProviders.name')}</span>
        <input class="sw-input" bind:value={name} placeholder="DeepSeek" autocomplete="off" />
      </label>

      <label class="fld">
        <span>{t('myProviders.baseUrl')}</span>
        <input class="sw-input" bind:value={baseUrl} placeholder="https://api.deepseek.com/v1" spellcheck="false" autocomplete="off" />
        {#if baseUrl.trim() && !isValidUrl(baseUrl.trim())}
          <span class="warn">{t('myProviders.errInvalidUrl')}</span>
        {/if}
      </label>

      <div class="fld">
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
        <span class="hint">{t('myProviders.connectViaHint')}</span>
      </div>

      {#if needsProfile}
        <div class="fld">
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
        <p class="warn">{t('myProviders.openaiNeedsRouter')}</p>
      {/if}

      <label class="fld">
        <span>{t('myProviders.model')}</span>
        <input class="sw-input" bind:value={model} placeholder="deepseek-chat" spellcheck="false" />
      </label>

      <label class="fld">
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
          <div class="fld">
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
            <span class="hint">{t('myProviders.protocolHint')}</span>
          </div>
          <label class="fld">
            <span>{t('myProviders.smallModel')}</span>
            <input class="sw-input" bind:value={smallModel} placeholder={t('myProviders.smallModelPlaceholder')} spellcheck="false" />
            <span class="hint">{t('myProviders.smallModelHint')}</span>
          </label>
          <label class="fld">
            <span>{t('myProviders.balanceUrl')}</span>
            <input class="sw-input" bind:value={balanceUrl} placeholder="https://api.deepseek.com/user/balance" spellcheck="false" autocomplete="off" />
            <span class="hint">{t('myProviders.balanceUrlHint')}</span>
          </label>
        </div>
      {/if}

      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel}>{t('myProviders.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!canSubmit} onclick={submit}>{t('myProviders.save')}</button>
      </div>
</ModalShell>

<style>
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
  .fld > span:first-child {
    display: block;
    margin-bottom: 6px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
  }
  .ddwrap :global(.dd),
  .ddwrap :global(.dd > button) {
    width: 100%;
  }
  .ddwrap :global(.dd > button) {
    justify-content: space-between;
    display: flex;
  }
  .hint {
    display: block;
    margin-top: 4px;
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
  }
  .warn {
    display: block;
    margin-top: 4px;
    color: var(--sw-warn);
    font-size: var(--sw-text-xs);
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
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-6);
  }
</style>
