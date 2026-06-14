<script lang="ts">
  import type { Component } from '$lib/ipc';
  import { glossaryText } from '$lib/glossary';
  import { t, locale, pUpdate } from '$lib/i18n';

  let {
    comp,
    status,
    busy,
    anyRunning,
    onCheck,
    onApply,
    onOpenForks
  }: {
    comp: Component;
    status: any;
    busy: boolean;
    anyRunning: boolean;
    onCheck: () => void;
    onApply: () => void;
    onOpenForks?: () => void;
  } = $props();

  // Forks card: one-line actionable summary + a jump to the Forks tab.
  let forkSummary = $derived.by(() => {
    if (comp.id !== 'forks') return null;
    const s = status?.summary;
    if (!s) return null;
    const parts: string[] = [];
    if (s.conflict > 0) parts.push(t('updates.forkConflicts', { count: s.conflict }));
    if (s.merged > 0) parts.push(t('updates.forkToDelete', { count: s.merged }));
    if (s.open > 0) parts.push(t('updates.forkOpenPr', { count: s.open }));
    return {
      needHands: s.needHands ?? 0,
      text: parts.length ? parts.join(' · ') : t('updates.forkAllSynced')
    };
  });

  function fmtTime(ts?: string) {
    if (!ts) return t('common.dash');
    try {
      const loc = locale.current === 'ru' ? 'ru-RU' : locale.current === 'zh' ? 'zh-CN' : 'en-US';
      return new Date(ts).toLocaleString(loc);
    } catch {
      return ts;
    }
  }

  // Read the unified status envelope (schemaVersion/status/counts), falling back
  // to the legacy plugins/forks field names for any not-yet-migrated writer.
  function countOf(s: any, key: 'changed' | 'failed') {
    if (s?.counts && typeof s.counts[key] === 'number') return s.counts[key] as number;
    const legacyArr = s?.[key];
    if (Array.isArray(legacyArr)) return legacyArr.length;
    const legacyNum = s?.[`plugins_${key}`];
    return typeof legacyNum === 'number' ? legacyNum : 0;
  }

  // Coarse health from the unified envelope.
  let health = $derived.by(() => {
    if (comp.lastJson === null) return { label: t('updates.healthNoStatus'), cls: 'badge-muted' };
    const s = status;
    if (!s) return { label: t('updates.healthNoData'), cls: 'badge-muted' };
    const changed = countOf(s, 'changed');
    const failed = countOf(s, 'failed');
    const st = s.status as string | undefined;
    if (st === 'error')
      return {
        label: failed > 0 ? t('updates.healthFailedCount', { count: failed }) : t('updates.healthError'),
        cls: 'badge-err'
      };
    if (st === 'held') return { label: t('updates.healthHeld'), cls: 'badge-muted' };
    if (failed > 0)
      return {
        label:
          failed === 1
            ? t('updates.healthNeedsAttentionOne', { count: failed })
            : t('updates.healthNeedsAttentionMany', { count: failed }),
        cls: 'badge-warn'
      };
    if (st === 'changes' || changed > 0)
      return { label: `${changed} ${pUpdate(changed)}`, cls: 'badge-ok' };
    return { label: t('updates.healthUpToDate'), cls: 'badge-ok' };
  });

  // Is an update actually available? Drives whether we show "Update" vs nothing.
  let updateInfo = $derived.by(() => {
    if (comp.lastJson === null || !status) return { known: false, count: 0 };
    const changed = countOf(status, 'changed');
    const st = status.status as string | undefined;
    return { known: true, count: changed, has: st === 'changes' || changed > 0 };
  });

  // Run duration: unified durationSec (number) or legacy "M:SS" string.
  let durationText = $derived.by(() => {
    if (typeof status?.durationSec === 'number') {
      const total = Math.round(status.durationSec);
      const m = Math.floor(total / 60);
      const sec = total % 60;
      return m > 0 ? `${m}:${String(sec).padStart(2, '0')}` : t('updates.durationSeconds', { count: sec });
    }
    return status?.duration ?? null;
  });
</script>

<div class="sw-card flex flex-col gap-sw-3" class:busy>
  <div class="flex items-start justify-between gap-sw-2">
    <div class="min-w-0">
      <h3 class="font-medium">{comp.name}</h3>
      <p class="text-sw-xs text-sw-text-muted">{comp.group}</p>
    </div>
    <span class="badge {health.cls} shrink-0">{health.label}</span>
  </div>

  {#if glossaryText(comp.id)}
    <p class="-mt-1 text-sw-xs leading-snug text-sw-text-secondary" title={glossaryText(comp.id)}>
      {glossaryText(comp.id)}
    </p>
  {/if}

  {#if comp.lastJson}
    <dl class="space-y-1 text-sw-sm text-sw-text-secondary">
      <div class="flex justify-between">
        <dt>{t('updates.lastRun')}</dt>
        <dd class="text-sw-text">{fmtTime(status?.timestamp)}</dd>
      </div>
      {#if durationText}
        <div class="flex justify-between">
          <dt>{t('updates.duration')}</dt>
          <dd class="text-sw-text">{durationText}</dd>
        </div>
      {/if}
    </dl>
  {/if}

  {#if forkSummary}
    <div class="flex items-center gap-sw-2 rounded-sw-md border border-sw-border p-sw-2 text-sw-xs">
      <span class="{forkSummary.needHands > 0 ? 'text-amber-400' : 'text-sw-text-secondary'}">
        {forkSummary.text}
      </span>
    </div>
  {/if}

  <div class="mt-auto flex gap-sw-2 pt-sw-2">
    <button class="sw-btn sw-btn-ghost flex-1" disabled={anyRunning} onclick={onCheck}
      title={t('updates.checkTip')}>
      {busy ? t('updates.checking') : t('updates.checkBtn')}
    </button>
    {#if comp.id === 'forks' && onOpenForks}
      <button class="sw-btn sw-btn-primary flex-1" disabled={anyRunning} onclick={onOpenForks}
        title={t('updates.openForksTip')}>
        {t('updates.openForksBtn')}
      </button>
    {/if}
    {#if comp.supportsApply}
      {#if updateInfo.has}
        <button class="sw-btn sw-btn-primary flex-1" disabled={anyRunning} onclick={onApply}
          title={t('updates.updateTip')}>
          {updateInfo.count > 1 ? t('updates.updateBtnCount', { count: updateInfo.count }) : t('updates.updateBtn')}
        </button>
      {:else if !updateInfo.known}
        <button class="sw-btn sw-btn-primary flex-1" disabled={anyRunning} onclick={onApply}
          title={t('updates.applyTip')}>
          {t('updates.applyBtn')}
        </button>
      {:else}
        <span class="flex-1 self-center text-center text-sw-xs text-sw-text-muted" title={t('updates.upToDateTip')}>{t('updates.upToDate')}</span>
      {/if}
    {/if}
  </div>
</div>

<style>
  .busy {
    border-color: var(--sw-border-focus);
    box-shadow: 0 0 16px var(--sw-accent-glow);
  }
</style>
