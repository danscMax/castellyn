<script lang="ts">
  // Rendered by +layout.svelte when this window is a detached per-monitor / popped-out window (its
  // label is not "main"). It reads the handoff spec stashed by the main window (by window label) and
  // mirrors the LIVE session(s) via TerminalPane's attach mode — no respawn. Panes here OWN their
  // sessions: closing a single pane (its ✕) ENDS that session; closing the WHOLE window returns every
  // live session to the main window instead of ending them (see closeWin). Emptying the window closes it.
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { t } from '$lib/i18n';
  import { takeDetach, type DetachPane } from '$lib/ipc';
  import { emit } from '@tauri-apps/api/event';
  import { markMoved } from '$lib/sessionMove';
  import TerminalPane from './TerminalPane.svelte';

  const win = getCurrentWindow();
  // Local stable key per pane (sessionId is absent for restored/spawn panes, so it can't be the key).
  let panes = $state<Array<DetachPane & { _key: string }>>([]);
  let loaded = $state(false);

  onMount(async () => {
    try {
      const spec = await takeDetach(win.label);
      panes = (spec?.panes ?? []).map((p, i) => ({ ...p, _key: String(i) }));
    } catch {
      panes = [];
    }
    loaded = true;
  });

  const cols = $derived(Math.max(1, Math.min(3, Math.ceil(Math.sqrt(panes.length)))));

  // Live session id per pane (paneKey → id), captured from TerminalPane's onIdChange. A pane RESTORED
  // from a saved monitor layout carries NO spec sessionId (it spawns fresh here), so without this its
  // "return to main" stayed disabled — the trapped-on-a-monitor bug (owner report 2026-07-06).
  let liveIds = $state<Record<string, string>>({});
  function setLiveId(key: string, id: string | null) {
    if (id) {
      liveIds = { ...liveIds, [key]: id };
    } else {
      const copy = { ...liveIds };
      delete copy[key];
      liveIds = copy;
    }
  }
  function closePane(key: string) {
    panes = panes.filter((p) => p._key !== key);
    setLiveId(key, null);
    if (!panes.length) win.close();
  }
  function returnPane(p: DetachPane & { _key: string }) {
    // Prefer the LIVE id (freshly spawned on restore); fall back to the spec id (panes moved here).
    const id = liveIds[p._key] ?? p.sessionId;
    if (!id) return;
    // Hand this LIVE session back to the main window (it re-attaches as the owner); our pane then
    // unmounts → detaches its own channel (markMoved → no kill). The session never restarts.
    emit('pane:add', { target: 'main', pane: { ...p, sessionId: id, owns: true } });
    markMoved(id);
    closePane(p._key);
  }
  async function closeWin() {
    // Panes here OWN their sessions (owns:true), so a bare win.close() would let each TerminalPane's
    // onDestroy sessionKill it — closing a monitor window must NOT silently end (paid) sessions. Hand
    // every live session back to the main window first (markMoved suppresses the kill; the emit lets
    // main re-adopt it, matching returnPane). Await delivery so main receives them before we close.
    await Promise.all(
      panes.map((p) => {
        const id = liveIds[p._key] ?? p.sessionId;
        if (!id) return Promise.resolve();
        markMoved(id);
        return emit('pane:add', { target: 'main', pane: { ...p, sessionId: id, owns: true } });
      })
    );
    win.close();
  }
</script>

<div class="detached">
  <div class="bar" data-tauri-drag-region>
    <span class="ttl">{!loaded ? 'Castellyn' : panes.length === 1 ? panes[0].title : `Castellyn · ${panes.length}`}</span>
    <button class="x" onclick={closeWin} aria-label={t('common.close')} title={t('common.close')}>✕</button>
  </div>
  <div class="body">
    {#if loaded && panes.length}
      <div class="grid" style="grid-template-columns: repeat({cols}, minmax(0, 1fr))">
        {#each panes as p (p._key)}
          <div class="cell">
            <TerminalPane
              profile={p.profile ?? ''}
              tool={p.tool}
              args={p.args ?? ''}
              cwd={p.cwd}
              attachId={p.sessionId}
              ownsSession={p.owns ?? true}
              displayName={p.title}
              paneKey={p._key}
              onIdChange={setLiveId}
              onClose={() => closePane(p._key)}
              onReturnToMain={(liveIds[p._key] ?? p.sessionId) ? () => returnPane(p) : undefined}
            />
          </div>
        {/each}
      </div>
    {:else if loaded}
      <div class="empty">{t('sessions.detachedEmpty')}</div>
    {/if}
  </div>
</div>

<style>
  .detached {
    display: flex;
    flex-direction: column;
    height: 100vh;
    /* V3: themable surface (was the phantom --sw-bg → always dark #080c18). */
    background: var(--sw-bg-primary);
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 28px;
    padding: 0 8px;
    background: var(--sw-bg-secondary);
    border-bottom: 1px solid var(--sw-border, #1c2840);
    user-select: none;
  }
  .ttl {
    font-size: 12px;
    color: var(--sw-text-secondary, #9fb0d0);
    font-family: 'Cascadia Code', 'Consolas', monospace;
  }
  .x {
    background: transparent;
    border: 0;
    color: var(--sw-text-muted, #6f7e9e);
    cursor: pointer;
    font-size: 13px;
    padding: 2px 8px;
  }
  .x:hover {
    color: var(--sw-text-primary, #fff);
  }
  .body {
    flex: 1;
    min-height: 0;
  }
  .grid {
    display: grid;
    gap: 4px;
    padding: 4px;
    height: 100%;
  }
  .cell {
    min-width: 0;
    min-height: 0;
  }
  .empty {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--sw-text-muted, #6f7e9e);
    font-size: 13px;
  }
</style>
