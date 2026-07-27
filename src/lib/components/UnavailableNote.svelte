<script lang="ts">
  // "We could not look" — distinct from "there is nothing here". Renders the localized reason,
  // an optional copyable command, an optional link, and the raw tool output behind a toggle.
  // Deliberately NOT an error: a missing external tool on a fresh machine is an ordinary state,
  // so this reads as an informational note, not a failure.
  import type { Unavailable } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { copyText } from '$lib/clipboard';
  import { Info, Copy, Check, ExternalLink } from '@lucide/svelte';

  let { info, onOpenUrl }: { info: Unavailable; onOpenUrl?: (url: string) => void } = $props();

  let copied = $state(false);
  let showDetail = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    if (!info.fixCommand || !(await copyText(info.fixCommand))) return;
    copied = true;
    // Reset the tick without stacking timers when the button is hammered.
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copied = false), 1600);
  }
</script>

<div class="unavail" role="note">
  <div class="head">
    <span class="ico" aria-hidden="true"><Info size={14} /></span>
    <span class="reason">{t(info.reason)}</span>
  </div>

  {#if info.fixCommand}
    <div class="fix">
      <code class="cmd">{info.fixCommand}</code>
      <button
        class="sw-btn text-sw-xs"
        onclick={copy}
        title={t('common.copy')}
        aria-label={t('common.copy')}
      >
        {#if copied}<Check size={13} aria-hidden="true" />{:else}<Copy size={13} aria-hidden="true" />{/if}
      </button>
    </div>
  {/if}

  <div class="links">
    {#if info.fixUrl && onOpenUrl}
      <button class="link" onclick={() => onOpenUrl?.(info.fixUrl!)}>
        <ExternalLink size={12} aria-hidden="true" /> {t('avail.howTo')}
      </button>
    {/if}
    {#if info.detail}
      <button class="link" onclick={() => (showDetail = !showDetail)} aria-expanded={showDetail}>
        {showDetail ? t('avail.hideDetail') : t('avail.showDetail')}
      </button>
    {/if}
  </div>

  {#if showDetail && info.detail}
    <!-- Raw tool output: never localized, never the primary message. -->
    <pre class="detail">{info.detail}</pre>
  {/if}
</div>

<style>
  .unavail {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--sw-border);
    border-left: 3px solid var(--sw-warn);
    border-radius: var(--sw-radius-md);
    background: var(--sw-bg-card);
    font-size: var(--sw-text-sm);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .ico {
    display: inline-flex;
    color: var(--sw-warn);
    flex: none;
  }
  .reason {
    color: var(--sw-text);
  }
  .fix {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    min-width: 0;
  }
  .cmd {
    flex: 1 1 auto;
    min-width: 0;
    overflow-x: auto;
    white-space: nowrap;
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
    background: var(--sw-bg-subtle);
    font-family: var(--sw-font-mono);
    font-size: var(--sw-text-xs);
    color: var(--sw-text);
  }
  .links {
    display: flex;
    gap: 0.75rem;
  }
  .links:empty {
    display: none;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0;
    border: 0;
    background: none;
    color: var(--sw-text-secondary);
    font: inherit;
    font-size: var(--sw-text-xs);
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:hover {
    color: var(--sw-text);
  }
  .detail {
    margin: 0;
    max-height: 8rem;
    overflow: auto;
    padding: 0.4rem 0.5rem;
    border-radius: 4px;
    background: var(--sw-bg-subtle);
    font-family: var(--sw-font-mono);
    font-size: var(--sw-text-xs);
    color: var(--sw-text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
