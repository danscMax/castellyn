<script lang="ts">
  import { t, plural, pFile } from '$lib/i18n';
  import { statusTextClass } from '$lib/statusColor';
  import { agentSummary } from '$lib/agentStatus.svelte';
  import { Check, X, Info } from '@lucide/svelte';
  import { profileHasMissingLink } from '$lib/attention';
  import StackDriftCard from './StackDriftCard.svelte';
  import StackGcCard from './StackGcCard.svelte';
  import type { SyncStatus, ConfigDriftStatus, ProfilesStatus, SchedulesStatus, StackService, StackDriftItem, GcItem } from '$lib/ipc';

  // USE-1: single-pane "is my Claude setup healthy?" overview. Pure aggregation of data the
  // other tabs already load; each chip deep-links to the tab that owns it.
  // F23: also a quick-action launchpad — the parent wires onAction(id) to its existing handlers.
  let {
    profiles = null,
    sync = null,
    drift = null,
    schedules = null,
    stack = null,
    sessionCount = null,
    stackDrift = null,
    gcItems = null,
    busy = false,
    components = null,
    statuses = null,
    onOpen,
    onRefresh,
    onReloadDrift,
    onReloadGc,
    onGcDelete,
    onAction
  }: {
    profiles: ProfilesStatus | null;
    sync: SyncStatus | null;
    drift: ConfigDriftStatus | null;
    schedules: SchedulesStatus | null;
    stack?: StackService[] | null;
    sessionCount?: number | null;
    /** Ф1: stack-ownership drift (owned by +page so the sidebar badge reads the same array). */
    stackDrift?: StackDriftItem[] | null;
    /** Ф2-GC: stack-garbage scan (null = not scanned yet / scanning). */
    gcItems?: GcItem[] | null;
    /** U3: the global run lock is held — quick actions would be silent no-ops, so disable them. */
    busy?: boolean;
    /** Redesign 2A: manifest components + their last-run envelopes feed the recent-runs strip. */
    components?: { id: string; name: string }[] | null;
    statuses?: Record<string, any> | null;
    onOpen: (id: string) => void;
    onRefresh?: () => void;
    onReloadDrift?: () => void | Promise<void>;
    onReloadGc?: () => void | Promise<void>;
    onGcDelete?: (ids: string[], labels: string[]) => void | Promise<void>;
    onAction?: (id: string) => void;
  } = $props();

  type Level = 'ok' | 'warn' | 'bad' | 'muted';
  // F23: a chip may carry one inline action (id routed through onAction, label shown on the button).
  type Chip = { key: string; tab: string; title: string; value: string; level: Level; action?: { id: string; label: string } };

  function ageStr(h: number) {
    if (h < 1) return t('common.minutesAgo', { n: Math.max(1, Math.round(h * 60)) });
    if (h < 48) return t('common.hoursAgo', { n: Math.round(h) });
    return t('common.daysAgo', { n: Math.round(h / 24) });
  }

  // z5_5: a subsystem that was never set up should read "not set up" (muted, deep-links to its tab)
  // rather than vanishing — an empty grid hides that sync/schedules/stack even exist.
  const notConfigured = (key: string, tab: string, title: string): Chip => ({
    key, tab, title, value: t('page.home_notConfigured'), level: 'muted'
  });

  const chips = $derived.by<Chip[]>(() => {
    const out: Chip[] = [];

    const b = profiles?.backup;
    out.push({
      key: 'backup', tab: 'backup', title: t('page.home_backup'),
      value: b && b.ageHours != null ? t('page.home_backupAge', { time: ageStr(b.ageHours) }) : t('page.home_noData'),
      level: !b || b.ageHours == null ? 'muted' : b.stale ? 'bad' : 'ok'
    });

    if (drift) {
      const d = drift.drifted ?? 0;
      const u = drift.unlinked ?? 0;
      out.push({
        key: 'drift', tab: 'sync', title: t('page.home_config'),
        value: d > 0 ? t('page.home_configDrifted', { n: d }) : u > 0 ? t('page.home_configUnlinked', { n: u }) : t('page.home_ok'),
        level: d > 0 ? 'bad' : u > 0 ? 'warn' : 'ok',
        // Drift/unlinked → one-click relink (parent runs the same confirmed action as the Sync tab).
        action: d > 0 || u > 0 ? { id: 'relink', label: t('page.home_relink') } : undefined
      });
    } else {
      out.push(notConfigured('drift', 'sync', t('page.home_config')));
    }

    if (profiles?.profiles?.length) {
      // Split the two distinct problems, using the SAME predicates as the sidebar badge
      // (attention.ts) so Home and the sidebar never show different counts:
      //  · broken  = dir exists but a shared link is missing  → repairable
      //  · missing = the profile dir doesn't exist            → needs Create (Profiles tab, #17)
      const broken = profiles.profiles.filter((p) => p.exists && profileHasMissingLink(p)).length;
      const missing = profiles.profiles.filter((p) => !p.exists).length;
      const brokenTxt = broken > 0
        ? `${broken} ${plural(broken, t('page.home_brokenLink_one'), t('page.home_brokenLink_few'), t('page.home_brokenLink_many'))}`
        : '';
      const missingTxt = missing > 0 ? `${missing} ${t('page.home_missingDirs')}` : '';
      const value = broken || missing
        ? [brokenTxt, missingTxt].filter(Boolean).join(' · ')
        : t('page.home_profilesOk', { n: profiles.profiles.length });
      out.push({
        key: 'profiles', tab: 'profiles', title: t('page.home_profiles'),
        value,
        level: broken || missing ? 'bad' : 'ok',
        // F23: one-click repair of existing broken profiles. A missing DIR needs Create (Repair exits
        // 1 on it), so no repair button then — clicking the chip deep-links to the Profiles tab.
        action: broken > 0 ? { id: 'repair-profiles', label: t('page.home_repairAll') } : undefined
      });
    }

    const conf = profiles?.syncConflicts?.count ?? 0;
    if (conf > 0) {
      out.push({
        key: 'conflicts', tab: 'sync', title: t('page.home_conflicts'), value: `${conf} ${pFile(conf)}`, level: 'warn',
        action: { id: 'clean-conflicts', label: t('page.home_cleanConflicts') }
      });
    }

    if (sync?.syncthing) {
      const st = sync.syncthing;
      const s = st.state;
      const bad = s === 'error' || s === 'outofsync';
      // Map known Syncthing states to localized labels (raw 'idle' was misread as "off");
      // reuse the sync.state* keys the Sync tab already owns. Unknown state → show as-is.
      const label =
        s === 'idle' ? t('sync.stateIdle')
        : s === 'syncing' ? t('sync.stateSyncing')
        : s === 'scanning' ? t('sync.stateScanning')
        : bad ? t('sync.stateError')
        : (s ?? t('common.dash'));
      out.push({
        key: 'sync', tab: 'sync', title: t('page.home_sync'),
        value: st.available ? label : t('page.home_syncOffline'),
        level: !st.available ? 'muted' : bad ? 'bad' : sync.stignoreMatches === false ? 'warn' : 'ok'
      });
    } else {
      out.push(notConfigured('sync', 'sync', t('page.home_sync')));
    }

    if (schedules?.tasks?.length) {
      // Three distinct states, kept separate so the chip doesn't lump "disabled" with "never created":
      //  · failing = created & last run errored   · missing = task never created   · off = created but disabled
      const failing = schedules.tasks.filter((x) => x.ok === false).length;
      const missing = schedules.tasks.filter((x) => !x.exists).length;
      const off = schedules.tasks.filter((x) => x.exists && !x.enabled).length;
      const parts = [
        missing > 0 ? t('page.home_tasksMissing', { n: missing }) : '',
        off > 0 ? t('page.home_tasksOff', { n: off }) : ''
      ].filter(Boolean).join(' · ');
      out.push({
        key: 'schedule', tab: 'schedule', title: t('page.home_tasks'),
        value: failing > 0 ? t('page.home_tasksFailing', { n: failing }) : parts || t('page.home_ok'),
        level: failing > 0 ? 'bad' : missing || off ? 'warn' : 'ok'
      });
    } else {
      out.push(notConfigured('schedule', 'schedule', t('page.home_tasks')));
    }

    if (stack?.length) {
      const enabled = stack.filter((s) => s.enabled);
      const up = enabled.filter((s) => s.running).length;
      out.push({
        key: 'stack', tab: 'providers', title: t('page.home_stack'),
        value: t('page.home_stackRunning', { up, total: enabled.length }),
        level: up === 0 ? 'muted' : up < enabled.length ? 'warn' : 'ok',
        action: up === 0 ? { id: 'start-stack', label: t('page.home_stackStart') } : { id: 'stop-stack', label: t('page.home_stackStop') }
      });
    } else {
      out.push(notConfigured('stack', 'providers', t('page.home_stack')));
    }

    if (sessionCount != null) {
      // herdr rollup on the cockpit: "N wait" (a decision is needed) beats a bare pane count.
      // agentSummary already feeds the sidebar badge — same source, same sessions.sum* keys.
      const parts = [
        agentSummary.blocked > 0 ? t('sessions.sumBlocked', { n: agentSummary.blocked }) : '',
        agentSummary.limited > 0 ? t('sessions.sumLimited', { n: agentSummary.limited }) : '',
        agentSummary.working > 0 ? t('sessions.sumWorking', { n: agentSummary.working }) : '',
        agentSummary.done > 0 ? t('sessions.sumDone', { n: agentSummary.done }) : ''
      ].filter(Boolean).join(' · ');
      out.push({
        key: 'sessions', tab: 'sessions', title: t('page.home_sessions'),
        value: parts || t('page.home_sessionsActive', { n: sessionCount }),
        level: agentSummary.blocked > 0 ? 'bad' : agentSummary.limited > 0 ? 'warn' : sessionCount > 0 ? 'ok' : 'muted'
      });
    }

    return out;
  });

  // Cockpit split (spec §4): problem chips get their own "needs attention" list with inline
  // actions; the healthy/informational rest stays a compact grid.
  const attention = $derived(chips.filter((c) => c.level === 'bad' || c.level === 'warn'));
  const calm = $derived(chips.filter((c) => c.level !== 'bad' && c.level !== 'warn'));
  const issues = $derived(attention.length);
  // U4: the quick-action bar shows ONE contextual stack button (like the stack chip), not both.
  const stackUp = $derived(((stack ?? []).filter((s) => s.enabled && s.running)).length);
  const overall = $derived(
    chips.some((c) => c.level === 'bad' || c.level === 'warn') ? 'warn' : chips.some((c) => c.level === 'ok') ? 'ok' : 'muted'
  );

  // Recent runs (spec §4): the freshest *.last.json envelopes, newest first. Data the Updates
  // tab already loads — pure re-aggregation, no new backend.
  type Run = { id: string; name: string; level: Level; when: string; summary: string };
  const runs = $derived.by<Run[]>(() => {
    if (!statuses) return [];
    const names = new Map((components ?? []).map((c) => [c.id, c.name]));
    return Object.entries(statuses)
      .map(([id, s]) => ({ id, s, ts: Date.parse(s?.timestamp ?? s?.generatedAt ?? '') }))
      .filter((x) => x.s && Number.isFinite(x.ts))
      .sort((a, b) => b.ts - a.ts)
      .slice(0, 6)
      .map(({ id, s, ts }) => ({
        id,
        name: names.get(id) ?? id,
        level: (s.status === 'error' ? 'bad' : s.status === 'held' || s.status === 'changes' ? 'warn' : 'ok') as Level,
        when: ageStr((Date.now() - ts) / 3_600_000),
        summary: typeof s.summary === 'string' ? s.summary : ''
      }));
  });
  const lastRun = $derived(runs[0] ?? null);

  // Theme-aware status text colour (shared source; light theme darkens to meet WCAG contrast).
  const color = (level: Level) => statusTextClass(level);
</script>

<div class="p-sw-6">
  <header class="mb-sw-4 flex items-start justify-between gap-sw-4">
    <div>
      <h1 class="text-lg font-semibold">{t('page.home_title')}</h1>
      <p class="text-sw-sm text-sw-text-secondary">{t('page.home_subtitle')}</p>
    </div>
    {#if onRefresh}
      <button class="sw-btn sw-btn-ghost shrink-0" onclick={onRefresh} title={t('common.refresh')}>{t('common.refresh')}</button>
    {/if}
  </header>

  <!-- Status strip (spec §4): overall verdict + freshest run + quick actions in one line. -->
  <div class="mb-sw-4 sw-card flex flex-wrap items-center gap-sw-3">
    <span class="badge {overall === 'ok' ? 'badge-ok' : overall === 'muted' ? 'badge-muted' : 'badge-warn'}">
      {overall === 'ok' ? t('page.home_allOk') : overall === 'muted' ? t('page.home_noData') : t('page.home_issues', { n: issues })}
    </span>
    {#if lastRun}
      <span class="text-sw-sm text-sw-text-muted">{t('page.home_lastMaint', { time: lastRun.when })}</span>
    {/if}
    {#if onAction}
      <!-- F23: quick actions — run the same parent handlers the dedicated tabs use. -->
      <span class="ml-auto flex flex-wrap gap-sw-2">
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} title={busy ? t('page.home_busy') : undefined} onclick={() => onAction('check-all')}>{t('page.home_checkAll')}</button>
        <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} title={busy ? t('page.home_busy') : undefined} onclick={() => onAction('refresh-forks')}>{t('page.home_refreshForks')}</button>
        {#if stackUp === 0}
          <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} title={busy ? t('page.home_busy') : undefined} onclick={() => onAction('start-stack')}>{t('page.home_stackStart')}</button>
        {:else}
          <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} title={busy ? t('page.home_busy') : undefined} onclick={() => onAction('stop-stack')}>{t('page.home_stackStop')}</button>
        {/if}
      </span>
    {/if}
  </div>

  <!-- Ф1: stack-ownership drift (plugin_sync hook + wiring, managed-settings). -->
  <StackDriftCard items={stackDrift} onReload={onReloadDrift} {busy} />

  <!-- Ф2-GC: stack garbage (stale plugin versions, temp_git, .bak, wrong-OS binaries). -->
  <StackGcCard items={gcItems} onReload={onReloadGc} onDelete={onGcDelete} {busy} />

  {#if overall === 'muted'}
    <!-- First run: no data yet → a 3-step orientation instead of an empty grid. -->
    <div class="sw-card mb-sw-4">
      <h2 class="section-title mb-sw-3">{t('page.home_firstSteps')}</h2>
      <ol class="flex flex-col gap-sw-2 text-sw-sm text-sw-text-secondary">
        <li><button class="link-btn" onclick={() => onOpen('settings')}>1. {t('page.home_step1')}</button></li>
        <li><button class="link-btn" disabled={busy} onclick={() => onAction?.('check-all')}>2. {t('page.home_step2')}</button></li>
        <li><button class="link-btn" onclick={() => onOpen('sessions')}>3. {t('page.home_step3')}</button></li>
      </ol>
    </div>
  {/if}

  {#if attention.length}
    <!-- Needs attention: only the problems, each with its inline fix + deep-link. -->
    <h2 class="section-title mb-sw-2">{t('page.home_attention')}</h2>
    <div class="mb-sw-4 flex flex-col gap-sw-2">
      {#each attention as c (c.key)}
        <div class="sw-card flex flex-wrap items-center gap-sw-3 !py-sw-2">
          <span class="text-sw-xs uppercase tracking-wide text-sw-text-muted w-28 shrink-0">{c.title}</span>
          <span class="font-medium {color(c.level)}">{c.value}</span>
          <span class="ml-auto flex gap-sw-2">
            {#if c.action && onAction}
              <button class="sw-btn sw-btn-ghost text-sw-xs" disabled={busy} onclick={() => onAction(c.action!.id)}>{c.action.label}</button>
            {/if}
            <button class="sw-btn sw-btn-ghost text-sw-xs" onclick={() => onOpen(c.tab)}>{t('common.open')}</button>
          </span>
        </div>
      {/each}
    </div>
  {/if}

  <div class="card-grid">
    {#each calm as c (c.key)}
      <div class="sw-card calm-card flex flex-col gap-1">
        <button class="flex flex-col gap-1 text-left" onclick={() => onOpen(c.tab)} title={t('common.open')}>
          <span class="text-sw-xs uppercase tracking-wide text-sw-text-muted">{c.title}</span>
          <span class="font-medium {color(c.level)}">{c.value}</span>
        </button>
        {#if c.action && onAction}
          <button class="sw-btn sw-btn-ghost text-sw-xs self-start" disabled={busy} onclick={() => onAction(c.action!.id)}>{c.action.label}</button>
        {/if}
      </div>
    {/each}
  </div>

  {#if runs.length}
    <!-- Recent runs: the freshest envelopes, one line each, deep-linking to the owner tab. -->
    <h2 class="section-title mt-sw-6 mb-sw-2">{t('page.home_recentRuns')}</h2>
    <div class="flex flex-col gap-sw-1">
      {#each runs as r (r.id)}
        <button class="run-row" onclick={() => onOpen(r.id === 'forks' ? 'forks' : 'updates')}>
          <!-- V6 icon language (same trio NotificationPanel uses) instead of ✓/✗/• text glyphs. -->
          <span class="{color(r.level)} run-ic">
            {#if r.level === 'ok'}<Check size={13} aria-hidden="true" />{:else if r.level === 'bad'}<X size={13} aria-hidden="true" />{:else}<Info size={13} aria-hidden="true" />{/if}
          </span>
          <span class="font-medium">{r.name}</span>
          <span class="text-sw-text-muted text-sw-sm">{r.when}</span>
          {#if r.summary}<span class="truncate text-sw-sm text-sw-text-secondary">{r.summary}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .link-btn {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .link-btn:hover:not(:disabled) {
    color: var(--sw-text-primary);
    text-decoration: underline;
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: var(--sw-space-3);
    padding: var(--sw-space-2) var(--sw-space-3);
    border: none;
    border-radius: var(--sw-radius-sm);
    background: transparent;
    color: var(--sw-text-primary);
    font-family: inherit;
    font-size: var(--sw-text-base);
    text-align: left;
    cursor: pointer;
    min-width: 0;
  }
  .run-row:hover {
    background: var(--sw-bg-hover);
  }
  .run-ic {
    display: inline-flex;
    flex-shrink: 0;
  }
  /* The whole calm card deep-links via its inner button — show that it's clickable. */
  .calm-card {
    transition: background 0.12s ease;
  }
  .calm-card:hover {
    background: var(--sw-bg-hover);
  }
</style>
