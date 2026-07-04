<script lang="ts">
  import { onMount } from 'svelte';
  import { readProfileUsage, type ProfileUsage } from '$lib/ipc';
  import { t } from '$lib/i18n';

  // Shows remaining Claude Code budget (5h + weekly) for a profile, refreshed every 60s. The
  // backend caches per profile, so polling is cheap. Renders nothing when not logged in / offline.
  // compact = minimal inline form (for the session pane bar): "5ч 90% · 7д 94%", no reset countdown.
  let { profile, compact = false }: { profile: string; compact?: boolean } = $props();

  let u = $state<ProfileUsage | null>(null);

  async function load() {
    try {
      u = await readProfileUsage(profile);
    } catch {
      u = null;
    }
  }
  onMount(() => {
    load();
    const id = setInterval(load, 60_000);
    return () => clearInterval(id);
  });

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
  const resetTxt = $derived(u ? until(u.sevenDayResetsAt) : '');
</script>

{#if r5 != null || r7 != null}
  {#if compact}
    <span class="flex items-center gap-x-sw-2 text-sw-xs whitespace-nowrap" title={t('profiles.usageTip')}>
      {#if r5 != null}<span><span class="text-sw-text-muted">{t('profiles.usage5h')}</span> <span class={color(r5)}>{r5}%</span></span>{/if}
      {#if r7 != null}<span><span class="text-sw-text-muted">{t('profiles.usage7d')}</span> <span class={color(r7)}>{r7}%</span></span>{/if}
    </span>
  {:else}
    <div class="flex flex-wrap items-center gap-x-sw-3 gap-y-1 text-sw-xs" title={t('profiles.usageTip')}>
      {#if r5 != null}
        <span><span class="text-sw-text-muted">{t('profiles.usage5h')}</span> <span class={color(r5)}>{r5}%</span></span>
      {/if}
      {#if r7 != null}
        <span><span class="text-sw-text-muted">{t('profiles.usage7d')}</span> <span class={color(r7)}>{r7}%</span></span>
      {/if}
      {#if resetTxt}
        <span class="text-sw-text-muted">{t('profiles.usageReset', { time: resetTxt })}</span>
      {/if}
    </div>
  {/if}
{/if}
