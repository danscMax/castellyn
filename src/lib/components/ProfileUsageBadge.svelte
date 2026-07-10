<script lang="ts">
  import { type ProfileUsage } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { usageStore, subscribeUsage } from '$lib/usagePoll.svelte';

  // Shows remaining Claude Code budget (5h + weekly) for a profile. P6: usage comes from a single
  // shared poll per profile (usagePoll.svelte), so 10 panes of one profile poll once, not 10 times.
  // Renders nothing when not logged in / offline. compact = minimal inline form for the pane bar.
  let { profile, compact = false }: { profile: string; compact?: boolean } = $props();

  // Subscribe to this profile's shared poll for the lifetime of the badge (re-subscribes if `profile`
  // changes; the cleanup stops the poll when the last subscriber leaves).
  $effect(() => subscribeUsage(profile));
  const u = $derived<ProfileUsage | null>(usageStore[profile] ?? null);

  // utilization → remaining % (what's left), clamped 0..100.
  const remain = (pct: number | null | undefined) =>
    pct == null ? null : Math.max(0, Math.min(100, Math.round(100 - pct)));

  // Remaining time until reset, as a localized duration. Reuses the providers.uptime* keys
  // (same {d}{h} / {h}{m} shape) so en/zh render correctly instead of hardcoded Russian units.
  function until(iso: string | null): string {
    if (!iso) return '';
    const ms = new Date(iso).getTime() - Date.now();
    if (!(ms > 0)) return '';
    const h = Math.floor(ms / 3_600_000);
    const d = Math.floor(h / 24);
    if (d >= 1) return t('providers.uptimeD', { d, h: h % 24 });
    const m = Math.floor((ms % 3_600_000) / 60_000);
    return t('providers.uptimeH', { h, m });
  }

  // Low remaining = warn/danger color.
  const color = (r: number | null) =>
    r == null ? '' : r <= 10 ? 'status-bad' : r <= 25 ? 'status-warn' : 'status-ok';

  const r5 = $derived(u ? remain(u.fiveHourPct) : null);
  const r7 = $derived(u ? remain(u.sevenDayPct) : null);
  // Model-scoped weekly cap: surfaced only when it is TIGHTER than the headline 7d, so the badge
  // never hides the number that actually gates the session (same rule as the Analytics table).
  const rScoped = $derived(
    u && u.scopedPct != null && u.scopedPct > (u.sevenDayPct ?? -1) ? remain(u.scopedPct) : null
  );
  const scopedName = $derived(u?.scopedLabel ?? '');
  const resetTxt = $derived(u ? until(u.sevenDayResetsAt) : '');
</script>

{#if r5 != null || r7 != null}
  {#if compact}
    <span class="flex items-center gap-x-sw-2 text-sw-xs whitespace-nowrap" title={t('profiles.usageTip')}>
      {#if r5 != null}<span><span class="text-sw-text-muted">{t('profiles.usage5h')}</span> <span class={color(r5)}>{r5}%</span></span>{/if}
      {#if r7 != null}<span><span class="text-sw-text-muted">{t('profiles.usage7d')}</span> <span class={color(r7)}>{r7}%</span></span>{/if}
      {#if rScoped != null}<span title={t('analytics.claudeScopedTip', { label: scopedName })}><span class="text-sw-text-muted">{scopedName}</span> <span class={color(rScoped)}>{rScoped}%</span></span>{/if}
    </span>
  {:else}
    <div class="flex flex-wrap items-center gap-x-sw-3 gap-y-1 text-sw-xs" title={t('profiles.usageTip')}>
      {#if r5 != null}
        <span><span class="text-sw-text-muted">{t('profiles.usage5h')}</span> <span class={color(r5)}>{r5}%</span></span>
      {/if}
      {#if r7 != null}
        <span><span class="text-sw-text-muted">{t('profiles.usage7d')}</span> <span class={color(r7)}>{r7}%</span></span>
      {/if}
      {#if rScoped != null}
        <span title={t('analytics.claudeScopedTip', { label: scopedName })}><span class="text-sw-text-muted">{scopedName}</span> <span class={color(rScoped)}>{rScoped}%</span></span>
      {/if}
      {#if resetTxt}
        <span class="text-sw-text-muted">{t('profiles.usageReset', { time: resetTxt })}</span>
      {/if}
    </div>
  {/if}
{/if}
