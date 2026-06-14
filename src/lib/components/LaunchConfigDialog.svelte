<script lang="ts">
  import type { ProfileLaunch } from '$lib/ipc';
  import { t, locale } from '$lib/i18n';
  import Toggle from './Toggle.svelte';

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

  async function apply() {
    await onSave(selection());
    onCancel();
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

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && onCancel()} />

{#if open && profile}
  <div class="overlay">
    <button type="button" class="backdrop" aria-label={t('profiles.lcClose')} onclick={onCancel}></button>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1">
      <h3>{t('profiles.lcTitle', { name: profile.name })}</h3>

      <div class="chk mb-sw-3">
        <Toggle bind:checked={lean} title={t('profiles.lcLeanToggle')} />
        <div>
          <div class="text-sw-sm text-sw-text-primary">{t('profiles.lcLeanHeading')}</div>
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
          <div class="fld">
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
          <div>{t('profiles.lcLeanResult')}<span class="font-mono text-sw-text-primary">{fmt(leanSize)}</span> {t('profiles.lcTokensUnit')}</div>
          <div>{t('profiles.lcFullResult')}<span class="font-mono text-sw-text-primary">{fmt(fullSize)}</span> {t('profiles.lcTokensUnit')}</div>
        </div>
        {#if measureErr}
          <p class="mt-sw-2 text-sw-xs" style="color:#f59e0b">{measureErr}</p>
        {/if}
        <p class="mt-sw-1 text-sw-xs text-sw-text-muted">
          {t('profiles.lcMeasureNote', { cmd: 'claude -p' })}
        </p>
      </div>

      <div class="row">
        <button class="sw-btn sw-btn-ghost" onclick={onCancel} title={t('profiles.lcCancelTip')}>{t('common.cancel')}</button>
        <button class="sw-btn sw-btn-primary" disabled={!!measuring} onclick={apply} title={t('profiles.lcApplyTip')}>{t('common.apply')}</button>
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
    width: min(520px, 94vw);
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
  .chk {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-6);
  }
</style>
