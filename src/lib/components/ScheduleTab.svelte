<script lang="ts">
  import type { SchedulesStatus, ScheduleAction } from '$lib/ipc';
  import { t, locale } from '$lib/i18n';

  let {
    data,
    running,
    onAction,
    onRefresh
  }: {
    data: SchedulesStatus | null;
    running: string | null;
    onAction: (action: ScheduleAction, id: string, time?: string) => void;
    onRefresh: () => void;
  } = $props();

  const busy = $derived(!!running);
  const tasks = $derived(data?.tasks ?? []);

  function fmtNext(ts: string | null) {
    if (!ts) return '—';
    try {
      const loc = locale.current === 'ru' ? 'ru-RU' : locale.current === 'zh' ? 'zh-CN' : 'en-US';
      return new Date(ts).toLocaleString(loc);
    } catch {
      return ts;
    }
  }
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('schedule.title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">
        {t('schedule.subtitle')}
      </p>
    </div>
    <button class="sw-btn sw-btn-ghost shrink-0" disabled={busy} onclick={onRefresh}
      title={t('schedule.refreshHint')}>
      {running === 'schedule' ? t('common.busy') : t('common.refresh')}
    </button>
  </header>

  {#if tasks.length}
    <div class="flex flex-col gap-sw-4">
      {#each tasks as task (task.id)}
        <div class="sw-card flex flex-col gap-sw-3">
          <div class="flex items-start justify-between gap-sw-2">
            <div>
              <h3 class="font-medium">{task.label}</h3>
              <p class="font-mono text-sw-xs text-sw-text-muted">{task.tn}</p>
            </div>
            {#if !task.exists}
              <span class="badge badge-muted" title={t('schedule.statusNotCreatedHint')}>{t('schedule.statusNotCreatedBadge')}</span>
            {:else if task.enabled}
              <span class="badge badge-ok" title={t('schedule.statusEnabledHint')}>{t('schedule.statusEnabledBadge')}</span>
            {:else}
              <span class="badge badge-warn" title={t('schedule.statusDisabledHint')}>{t('schedule.statusDisabledBadge')}</span>
            {/if}
          </div>

          {#if task.exists}
            <dl class="grid grid-cols-2 gap-x-sw-6 gap-y-1 text-sw-sm">
              <div>
                <dt class="text-sw-xs text-sw-text-muted">{t('schedule.timeDaily')}</dt>
                <dd class="text-sw-text">{task.time ?? '—'}</dd>
              </div>
              <div>
                <dt class="text-sw-xs text-sw-text-muted">{t('schedule.nextRun')}</dt>
                <dd class="text-sw-text">{fmtNext(task.nextRun)}</dd>
              </div>
            </dl>
          {:else}
            <p class="text-sw-sm text-sw-text-secondary">
              {t('schedule.willCreate', { time: task.defaultTime })}
            </p>
          {/if}

          <div class="mt-auto flex flex-wrap gap-sw-2 border-t border-sw-border pt-sw-2">
            {#if !task.exists}
              <button class="sw-btn sw-btn-primary text-sw-xs" disabled={busy} onclick={() => onAction('create', task.id)}
                title={t('schedule.createScheduleHint', { time: task.defaultTime })}>{t('schedule.createSchedule')}</button>
            {:else}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction('run', task.id)}
                title={t('schedule.runNowHint')}>{t('schedule.runNow')}</button>
              {#if task.enabled}
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction('disable', task.id)}
                  title={t('schedule.disableHint')}>{t('common.disable')}</button>
              {:else}
                <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction('enable', task.id)}
                  title={t('schedule.enableHint')}>{t('common.enable')}</button>
              {/if}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction('delete', task.id)}
                title={t('schedule.deleteHint')}>{t('common.delete')}</button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="grid place-items-center py-sw-6 text-center text-sw-text-muted">
      <div>
        <div class="mb-sw-2 text-2xl">🕒</div>
        <div class="font-medium text-sw-text">{t('schedule.emptyTitle')}</div>
        <div class="text-sw-sm">{t('schedule.emptyHint')}</div>
      </div>
    </div>
  {/if}
</div>
