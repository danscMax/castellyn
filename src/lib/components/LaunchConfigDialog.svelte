<script lang="ts">
  import type { ProfileLaunch } from '$lib/ipc';
  import { t, locale } from '$lib/i18n';
  import Toggle from './Toggle.svelte';
  import ModalShell from './ModalShell.svelte';

  let {
    open,
    profile,
    availableMcp = [],
    onSave,
    onMeasure,
    onCancel
  }: {
    open: boolean;
    profile: ProfileLaunch | null;
    availableMcp?: string[];
    onSave: (v: { mode: 'full' | 'lean'; mcp: string[]; claudeMd: boolean }) => Promise<void>;
    onMeasure: (lean: boolean) => Promise<number>;
    onCancel: () => void;
  } = $props();

  let lean = $state(false);
  let mcpSel = $state<Record<string, boolean>>({});
  let claudeMd = $state(false);
  let seeded = '';

  let leanSize = $state<number | null>(null);
  let fullSize = $state<number | null>(null);
  let measuring = $state<'lean' | 'full' | null>(null);
  let measureErr = $state('');

  const tokenAuth = $derived(profile?.tokenAuth ?? false);

  $effect(() => {
    const key = `${open}:${profile?.name ?? ''}`;
    if (open && profile && key !== seeded) {
      seeded = key;
      lean = profile.mode === 'lean';
      claudeMd = profile.claudeMd;
      mcpSel = Object.fromEntries(availableMcp.map((m) => [m, profile.mcp.includes(m)]));
      leanSize = null;
      fullSize = null;
      measureErr = '';
      measuring = null;
    }
  });

  function selection(): { mode: 'full' | 'lean'; mcp: string[]; claudeMd: boolean } {
    return {
      mode: lean ? 'lean' : 'full',
      mcp: availableMcp.filter((m) => mcpSel[m]),
      claudeMd
    };
  }

  // R7: gate the in-flight save like the sibling measure() does — a double-click used to fire
  // onSave twice before the dialog closed.
  let applying = $state(false);
  let applyErr = $state('');
  async function apply() {
    if (applying) return;
    applying = true;
    applyErr = '';
    try {
      await onSave(selection());
      onCancel(); // R5: only close once the save actually succeeded
    } catch (e) {
      // R5: a rejected save must keep the dialog open (was closing as if saved) and say why.
      applyErr = String((e as { message?: string })?.message ?? e);
    } finally {
      applying = false;
    }
  }

  // Measure always reflects what's on screen: persist the selection first, then measure.
  async function measure(which: 'lean' | 'full') {
    measureErr = '';
    measuring = which;
    try {
      await onSave(selection());
      const tokens = await onMeasure(which === 'lean');
      if (which === 'lean') leanSize = tokens;
      else fullSize = tokens;
    } catch (e) {
      measureErr = String(e);
    } finally {
      measuring = null;
    }
  }

  function fmt(n: number | null) {
    if (n === null) return '—';
    const loc = locale.current === 'ru' ? 'ru-RU' : locale.current === 'zh' ? 'zh-CN' : 'en-US';
    return n.toLocaleString(loc);
  }
</script>

<ModalShell open={open && !!profile} onClose={onCancel} size="md">
  {#if profile}
      <h3 class="dlg-h">{t('profiles.lcTitle', { name: profile.name })}</h3>

      <div class="chk mb-sw-3">
        <Toggle bind:checked={lean} title={t('profiles.lcLeanToggle')} />
        <div>
          <div class="text-sw-sm text-sw-text">{t('profiles.lcLeanHeading')}</div>
          <div class="text-sw-xs text-sw-text-muted">
            {t('profiles.lcLeanDesc')}
          </div>
        </div>
      </div>

      {#if lean}
        <p class="mb-sw-3 rounded-sw-md border border-sw-border p-sw-2 text-sw-xs text-sw-text-secondary">
          {#if tokenAuth}
            {t('profiles.lcBareNote', { bare: '--bare' })}
          {:else}
            {t('profiles.lcSafeModeNote', { safeMode: '--safe-mode' })}
          {/if}
        </p>

        {#if tokenAuth}
          <div class="dlg-fld">
            <span>{t('profiles.lcMcpLabel')}</span>
            {#if availableMcp.length}
              <div class="grid grid-cols-2 gap-1">
                {#each availableMcp as m (m)}
                  <label class="flex items-center gap-sw-2 text-sw-xs">
                    <Toggle bind:checked={mcpSel[m]} title={m} />
                    <span class="font-mono">{m}</span>
                  </label>
                {/each}
              </div>
            {:else}
              <p class="text-sw-xs text-sw-text-muted">{t('profiles.lcMcpEmpty')}</p>
            {/if}
          </div>

          <div class="chk mb-sw-3">
            <Toggle bind:checked={claudeMd} title={t('profiles.lcClaudeMdToggle')} />
            <span class="text-sw-xs text-sw-text-secondary">{t('profiles.lcClaudeMd')}</span>
          </div>
        {/if}
      {/if}

      <div class="rounded-sw-md border border-sw-border p-sw-2">
        <div class="mb-sw-2 flex items-center justify-between">
          <span class="text-sw-xs font-medium text-sw-text-secondary">{t('profiles.lcSizeLabel')}</span>
          <div class="flex gap-sw-2">
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!!measuring} onclick={() => measure('lean')}
              title={t('profiles.lcMeasureLeanTip')}>
              {measuring === 'lean' ? t('profiles.lcMeasuring') : t('profiles.lcMeasureLean')}
            </button>
            <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={!!measuring} onclick={() => measure('full')}
              title={t('profiles.lcMeasureFullTip')}>
              {measuring === 'full' ? t('profiles.lcMeasuring') : t('profiles.lcMeasureFull')}
            </button>
          </div>
        </div>
        <div class="grid grid-cols-2 gap-sw-2 text-sw-sm">
          <div>{t('profiles.lcLeanResult')}<span class="font-mono text-sw-text">{fmt(leanSize)}</span> {t('profiles.lcTokensUnit')}</div>
          <div>{t('profiles.lcFullResult')}<span class="font-mono text-sw-text">{fmt(fullSize)}</span> {t('profiles.lcTokensUnit')}</div>
        </div>
        {#if measureErr}
          <p class="mt-sw-2 text-sw-xs status-warn">{measureErr}</p>
        {/if}
        <p class="mt-sw-1 text-sw-xs text-sw-text-muted">
          {t('profiles.lcMeasureNote', { cmd: 'claude -p' })}
        </p>
      </div>

      {#if applyErr}
        <p class="mt-sw-2 text-sw-xs status-bad">{applyErr}</p>
      {/if}

      <div class="dlg-row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('profiles.lcCancelTip')}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!!measuring || applying} onclick={apply} title={t('profiles.lcApplyTip')}>{t('common.apply')}</button>
      </div>
  {/if}
</ModalShell>

<style>
  .chk {
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
