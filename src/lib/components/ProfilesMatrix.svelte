<script lang="ts">
  // Ф2.5 per-profile matrix: rows = profiles, columns = provider / proxy / shared folders. Edits
  // accumulate locally (dirty rows highlighted); "Apply" opens a preview confirm, then +page runs
  // the change-set sequentially (provider → proxy → folders, then relink) and we re-read to verify.
  import type { EngineStatus, MyProvider, MatrixRow, MatrixApply } from '$lib/ipc';
  import { readProfileMatrix } from '$lib/ipc';
  import { t } from '$lib/i18n';
  import { profileDotColor } from '$lib/statusColor';
  import { isValidHttpUrl } from '$lib/url';
  import { anchored } from '$lib/floating';
  import Toggle from './Toggle.svelte';
  import Select from './Select.svelte';
  import ModalShell from './ModalShell.svelte';

  let {
    engines = [],
    myProviders = [],
    running,
    onApplyMatrix,
    onMcpDeployProfile,
    onMcpRemoveExtra,
    mcpTick = 0
  }: {
    engines?: EngineStatus[] | null;
    myProviders?: MyProvider[] | null;
    running: string | null;
    onApplyMatrix: (changes: MatrixApply) => Promise<{ skipped: string[] }>;
    // MCP reconcile actions (fire-and-forget: they stream / confirm, then reloadMcp bumps mcpTick).
    onMcpDeployProfile: (profile: string) => void;
    onMcpRemoveExtra: (server: string, profile: string) => void;
    // Bumped by +page after any MCP reload → re-read matrix rows so mcp facts refresh (draft kept).
    mcpTick?: number;
  } = $props();

  const busy = $derived(!!running);

  // --- Baseline (server truth) + local draft overlay -----------------------------------------
  // plugins = per-id explicit override (true/false); absent id = no draft edit (baseline stands).
  type Draft = { provider: string; proxy: string; folders: string[]; plugins: Record<string, boolean> };
  let rows = $state<MatrixRow[]>([]);
  let loaded = $state(false);
  let loadErr = $state('');
  let draft = $state<Record<string, Draft>>({});
  let applying = $state(false);

  // '' base URL = OAuth/subscription (no custom provider). Draft mirrors that convention.
  function seedRow(r: MatrixRow): Draft {
    return {
      provider: r.provider.baseUrl ?? '',
      proxy: r.proxy ?? '',
      folders: r.folders.filter((f) => f.desired).map((f) => f.name),
      plugins: {}
    };
  }
  function seed(list: MatrixRow[]) {
    const d: Record<string, Draft> = {};
    for (const r of list) d[r.name] = seedRow(r);
    draft = d;
  }
  // reseed=false: refresh server truth but keep the current draft (used for the mcpTick re-read,
  // where only mcp facts changed and the user may have unsaved provider/plugin edits to preserve).
  // Rows new since the seed (profile created meanwhile) still get a fresh draft entry — the row
  // markup dereferences draft[r.name] unguarded.
  async function load(reseed = true) {
    try {
      const list = await readProfileMatrix();
      rows = list;
      if (reseed) seed(list);
      else for (const r of list) if (!draft[r.name]) draft[r.name] = seedRow(r);
      loadErr = '';
    } catch (e) {
      loadErr = String(e);
    } finally {
      loaded = true;
    }
  }
  $effect(() => {
    // Load once when the tab first renders this section.
    if (!loaded) load();
  });
  // An MCP deploy/remove elsewhere bumps mcpTick → re-read mcp facts without dropping the draft.
  // Plain (non-reactive) let: null until the first post-load run seeds it, so no spurious re-read.
  let lastTick: number | null = null;
  $effect(() => {
    const tick = mcpTick;
    if (!loaded) return;
    if (lastTick === null) {
      lastTick = tick;
      return;
    }
    if (tick === lastTick) return;
    lastTick = tick;
    load(false);
  });

  // --- Provider options (reused: anthropic engines + saved custom providers + OAuth) -----------
  const providerOptions = $derived([
    { value: '', label: t('profiles.matrixProviderOauth') },
    ...(engines ?? [])
      .filter((e) => e.protocol === 'anthropic' && e.baseUrl)
      .map((e) => ({ value: e.baseUrl, label: e.name })),
    ...(myProviders ?? [])
      .filter((m) => m.baseUrl && !(engines ?? []).some((e) => e.baseUrl === m.baseUrl))
      .map((m) => ({ value: m.baseUrl, label: m.name }))
  ]);
  function providerLabel(baseUrl: string): string {
    if (!baseUrl) return t('profiles.matrixProviderOauth');
    const e = (engines ?? []).find((x) => x.baseUrl === baseUrl);
    if (e) return e.name;
    const m = (myProviders ?? []).find((x) => x.baseUrl === baseUrl);
    if (m) return m.name;
    try {
      return new URL(baseUrl).host;
    } catch {
      return baseUrl;
    }
  }
  // Model fields for a chosen provider come from a saved custom provider; local engines carry none.
  function modelFor(baseUrl: string): { model: string; smallModel: string } {
    const m = (myProviders ?? []).find((x) => x.baseUrl === baseUrl);
    return { model: m?.model ?? '', smallModel: m?.smallModel ?? '' };
  }

  // --- Dirty tracking ------------------------------------------------------------------------
  const rowByName = $derived(new Map(rows.map((r) => [r.name, r])));
  function baseFolders(r: MatrixRow): string[] {
    return r.folders.filter((f) => f.desired).map((f) => f.name);
  }
  function eqSet(a: string[], b: string[]): boolean {
    if (a.length !== b.length) return false;
    const s = new Set(a);
    return b.every((x) => s.has(x));
  }
  function providerChanged(name: string): boolean {
    const r = rowByName.get(name);
    return !!r && (draft[name]?.provider ?? '') !== (r.provider.baseUrl ?? '');
  }
  function proxyChanged(name: string): boolean {
    const r = rowByName.get(name);
    return !!r && (draft[name]?.proxy ?? '').trim() !== (r.proxy ?? '');
  }
  function foldersChanged(name: string): boolean {
    const r = rowByName.get(name);
    return !!r && !eqSet(draft[name]?.folders ?? [], baseFolders(r));
  }
  // --- Plugins (per-profile enabledPlugins override) -----------------------------------------
  // Effective on = draft override if set, else the stored state is 'on'.
  function pluginOn(name: string, p: { id: string; state: string }): boolean {
    const ov = draft[name]?.plugins[p.id];
    return ov === undefined ? p.state === 'on' : ov;
  }
  // Dirty = the override changes what's STORED. unset has no stored bool, so any explicit override
  // (true OR false — the false being a deliberate opt-out) is a change; on/off only flip.
  function pluginDirty(name: string, p: { id: string; state: string }): boolean {
    const ov = draft[name]?.plugins[p.id];
    if (ov === undefined) return false;
    if (p.state === 'on') return ov === false;
    if (p.state === 'off') return ov === true;
    return true; // unset
  }
  function pluginsChanged(name: string): boolean {
    const r = rowByName.get(name);
    return !!r && r.plugins.some((p) => pluginDirty(name, p));
  }
  function togglePlugin(name: string, id: string, on: boolean) {
    draft[name] = { ...draft[name], plugins: { ...draft[name].plugins, [id]: on } };
  }
  function pluginOnCount(name: string): number {
    const r = rowByName.get(name);
    return r ? r.plugins.filter((p) => pluginOn(name, p)).length : 0;
  }
  // short label: strip any @version tail.
  const pluginShort = (id: string): string => id.split('@')[0];

  // --- MCP facts (read-only reconcile status) ------------------------------------------------
  function mcpMissing(r: MatrixRow): string[] {
    const have = new Set(r.mcp.deployed);
    return r.mcp.canon.filter((c) => !have.has(c));
  }
  function mcpWarn(r: MatrixRow): boolean {
    return mcpMissing(r).length > 0 || r.mcp.extras.length > 0;
  }

  function rowDirty(name: string): boolean {
    return providerChanged(name) || proxyChanged(name) || foldersChanged(name) || pluginsChanged(name);
  }
  const dirtyNames = $derived(rows.map((r) => r.name).filter((n) => rowDirty(n)));
  // A proxy edit that isn't a clear must be a valid http(s)/socks5 URL, else Apply is blocked.
  function proxyValid(name: string): boolean {
    const v = (draft[name]?.proxy ?? '').trim();
    return v === '' || isValidHttpUrl(v) || /^socks5:\/\//i.test(v);
  }
  const anyInvalid = $derived(dirtyNames.some((n) => !proxyValid(n)));
  const canApply = $derived(dirtyNames.length > 0 && !anyInvalid && !busy && !applying);

  // --- Popover (folders / plugins / mcp — one at a time, anchored to the clicked chip) ---------
  type PopKind = 'folders' | 'plugins' | 'mcp';
  let popFor = $state<string | null>(null);
  let popKind = $state<PopKind>('folders');
  let popAnchor = $state<HTMLElement | null>(null);
  function togglePop(name: string, kind: PopKind, el: HTMLElement) {
    if (popFor === name && popKind === kind) {
      popFor = null;
      return;
    }
    popAnchor = el;
    popKind = kind;
    popFor = name;
  }
  function toggleFolder(name: string, folder: string, on: boolean) {
    const cur = draft[name].folders;
    draft[name] = {
      ...draft[name],
      folders: on ? [...new Set([...cur, folder])] : cur.filter((f) => f !== folder)
    };
  }
  // Amber the chip when the profile isn't fully linked (needs attention / relink).
  function folderWarn(r: MatrixRow, name: string): boolean {
    const sel = draft[name]?.folders ?? [];
    if (sel.length < r.folders.length) return true;
    return r.folders.some((f) => f.desired && f.actual !== 'linked');
  }

  // --- Preview + apply -----------------------------------------------------------------------
  type Chg = { who: string; cat: string; text: string };
  const preview = $derived.by<Chg[]>(() => {
    const out: Chg[] = [];
    for (const r of rows) {
      const d = draft[r.name];
      if (!d) continue;
      if (providerChanged(r.name)) {
        out.push({
          who: r.name,
          cat: t('profiles.matrixCatProvider'),
          text: `${providerLabel(r.provider.baseUrl ?? '')} → ${providerLabel(d.provider)}`
        });
      }
      if (proxyChanged(r.name)) {
        const from = r.proxy || t('profiles.matrixProxyNone');
        const to = d.proxy.trim() || t('profiles.matrixProxyNone');
        out.push({ who: r.name, cat: t('profiles.matrixCatProxy'), text: `${from} → ${to}` });
      }
      if (foldersChanged(r.name)) {
        const before = new Set(baseFolders(r));
        const after = new Set(d.folders);
        const added = [...after].filter((f) => !before.has(f));
        const removed = [...before].filter((f) => !after.has(f));
        const parts = [...removed.map((f) => `−${f}`), ...added.map((f) => `+${f}`)];
        out.push({ who: r.name, cat: t('profiles.matrixCatFolders'), text: parts.join(', ') });
      }
      if (pluginsChanged(r.name)) {
        const on = r.plugins.filter((p) => pluginDirty(r.name, p) && d.plugins[p.id] === true);
        const off = r.plugins.filter((p) => pluginDirty(r.name, p) && d.plugins[p.id] === false);
        const parts = [...off.map((p) => `−${pluginShort(p.id)}`), ...on.map((p) => `+${pluginShort(p.id)}`)];
        out.push({ who: r.name, cat: t('profiles.matrixCatPlugins'), text: parts.join(', ') });
      }
    }
    return out;
  });

  function buildChanges(): MatrixApply {
    const providers: MatrixApply['providers'] = [];
    const proxies: MatrixApply['proxies'] = [];
    const folders: MatrixApply['folders'] = [];
    const plugins: MatrixApply['plugins'] = [];
    for (const r of rows) {
      const d = draft[r.name];
      if (!d) continue;
      if (providerChanged(r.name)) {
        const base = d.provider || null;
        const m = base ? modelFor(base) : { model: '', smallModel: '' };
        providers.push({ name: r.name, baseUrl: base, model: m.model, smallModel: m.smallModel });
      }
      if (proxyChanged(r.name)) proxies.push({ name: r.name, url: d.proxy.trim() });
      if (foldersChanged(r.name)) folders.push({ name: r.name, folders: d.folders });
      if (pluginsChanged(r.name)) {
        const enable = r.plugins.filter((p) => pluginDirty(r.name, p) && d.plugins[p.id] === true).map((p) => p.id);
        const disable = r.plugins.filter((p) => pluginDirty(r.name, p) && d.plugins[p.id] === false).map((p) => p.id);
        plugins.push({ name: r.name, enable, disable });
      }
    }
    return { providers, proxies, folders, plugins };
  }

  let previewOpen = $state(false);
  function openPreview() {
    if (!canApply) return;
    previewOpen = true;
  }
  function resetDraft() {
    seed(rows);
  }
  async function confirmApply() {
    previewOpen = false;
    applying = true;
    let ok = true;
    try {
      await onApplyMatrix(buildChanges());
    } catch {
      ok = false; // +page surfaces the error toast
    } finally {
      // Re-read to verify actual state. Success → reseed (everything applied, draft = baseline).
      // Failure → keep the draft overlay (load(false)): rows already applied auto-clear because
      // the refreshed baseline now matches them, failed/unapplied rows stay dirty for a retry.
      await load(ok);
      applying = false;
    }
  }
</script>

<section class="mt-sw-6">
  <div class="mb-sw-2 flex items-baseline justify-between gap-sw-3">
    <h2 class="text-base font-semibold">{t('profiles.matrixTitle')}</h2>
    <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixHint')}</span>
  </div>

  {#if loadErr}
    <p class="text-sw-sm status-bad">{loadErr}</p>
  {:else if !loaded}
    <div class="flex flex-col gap-sw-2">
      {#each Array(3) as _, i (i)}<div class="skeleton" style="height:2.4rem"></div>{/each}
    </div>
  {:else if rows.length === 0}
    <p class="text-sw-sm text-sw-text-muted">{t('profiles.matrixEmpty')}</p>
  {:else}
    <!-- z5_10: pb-16 clears the sticky Apply bar so the last rows can scroll above it, not under it. -->
    <div class="sw-card overflow-x-auto p-0 pb-16">
      <table class="mx">
        <thead>
          <tr>
            <th style="width:18%">{t('profiles.colName')}</th>
            <th style="width:22%">{t('profiles.colProvider')}</th>
            <th style="width:18%">{t('profiles.matrixColProxy')}</th>
            <th style="width:14%">{t('profiles.matrixColFolders')}</th>
            <th style="width:14%">{t('profiles.matrixColPlugins')}</th>
            <th style="width:14%">{t('profiles.matrixColMcp')}</th>
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.name)}
            {@const d = draft[r.name]}
            {@const dirty = rowDirty(r.name)}
            <tr class:dirty>
              <td>
                <span class="flex items-center gap-sw-2">
                  <span class="h-2.5 w-2.5 shrink-0 rounded-full" style="background:{profileDotColor(r.color)}"></span>
                  <span class="min-w-0">
                    <span class="flex items-center gap-sw-1 font-medium">
                      <span class="truncate" title={r.name}>{r.name}</span>
                      {#if dirty}<span class="unsaved-pill" title={t('profiles.matrixDirtyTip')}>{t('profiles.matrixUnsaved')}</span>{/if}
                    </span>
                    {#if r.description}<span class="block truncate text-sw-xs text-sw-text-muted" title={r.description}>{r.description}</span>{/if}
                  </span>
                </span>
              </td>
              <td>
                <Select
                  bind:value={d.provider}
                  options={providerOptions}
                  disabled={busy || applying}
                />
              </td>
              <td>
                <input
                  class="sw-input text-sw-sm"
                  bind:value={d.proxy}
                  placeholder={t('profiles.matrixProxyNone')}
                  spellcheck="false"
                  autocomplete="off"
                  disabled={busy || applying}
                  title={t('profiles.matrixProxyTip')}
                />
                {#if !proxyValid(r.name)}<span class="warn">{t('profiles.matrixProxyInvalid')}</span>{/if}
              </td>
              <td>
                <button
                  type="button"
                  class="chip"
                  class:warn={folderWarn(r, r.name)}
                  disabled={busy || applying}
                  onclick={(e) => togglePop(r.name, 'folders', e.currentTarget)}
                  title={t('profiles.matrixFoldersTip')}
                >
                  {d.folders.length}/{r.folders.length}
                </button>
              </td>
              <td>
                <button
                  type="button"
                  class="chip"
                  class:dirtychip={pluginsChanged(r.name)}
                  disabled={busy || applying}
                  onclick={(e) => togglePop(r.name, 'plugins', e.currentTarget)}
                  title={t('profiles.matrixPluginsTip')}
                >
                  {pluginOnCount(r.name)}/{r.plugins.length}
                </button>
              </td>
              <td>
                <button
                  type="button"
                  class="chip"
                  class:warn={mcpWarn(r)}
                  disabled={busy}
                  onclick={(e) => togglePop(r.name, 'mcp', e.currentTarget)}
                  title={t('profiles.matrixMcpTip')}
                >
                  {r.mcp.deployed.length}/{r.mcp.canon.length}{#if r.mcp.extras.length}&nbsp;+{r.mcp.extras.length}{/if}
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>

    {#if popFor && popAnchor}
      {@const r = rowByName.get(popFor)}
      {#if r}
        <div class="popover" use:anchored={{ anchor: popAnchor, onOutside: () => (popFor = null) }}>
          {#if popKind === 'folders'}
            {#each r.folders as f (f.name)}
              <label class="poprow">
                <Toggle
                  checked={draft[popFor].folders.includes(f.name)}
                  disabled={busy || applying}
                  onCheckedChange={(v) => toggleFolder(popFor!, f.name, v)}
                  title={f.name}
                />
                <span class="font-mono text-sw-xs">{f.name}</span>
                {#if f.desired && f.actual !== 'linked'}
                  <span class="status-warn text-sw-xs" title={t('profiles.matrixFolderRealTip')}>{f.actual === 'real' ? t('profiles.matrixFolderReal') : t('profiles.matrixFolderMissing')}</span>
                {/if}
              </label>
            {/each}
            <div class="warnnote">{t('profiles.matrixRelinkNote')}</div>
          {:else if popKind === 'plugins'}
            {#each r.plugins as p (p.id)}
              <label class="poprow">
                <Toggle
                  checked={pluginOn(popFor, p)}
                  disabled={busy || applying}
                  onCheckedChange={(v) => togglePlugin(popFor!, p.id, v)}
                  title={p.id}
                />
                <span class="min-w-0 flex-1 truncate text-sw-xs" title={p.id}>{pluginShort(p.id)}</span>
                {#if p.state === 'unset' && draft[popFor].plugins[p.id] === undefined}
                  <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixPluginInherited')}</span>
                {:else if p.state === 'off'}
                  <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixPluginOff')}</span>
                {/if}
              </label>
            {/each}
          {:else}
            {@const missing = mcpMissing(r)}
            {#if missing.length}
              <div class="popsec">{t('profiles.matrixMcpMissing')}</div>
              <div class="mcprow">
                <span class="min-w-0 flex-1 break-words font-mono text-sw-xs">{missing.join(', ')}</span>
                <button
                  type="button"
                  class="mcpbtn"
                  disabled={busy}
                  onclick={() => onMcpDeployProfile(popFor!)}
                >{t('profiles.matrixMcpDeployBtn')}</button>
              </div>
            {/if}
            {#if r.mcp.extras.length}
              <div class="popsec">{t('profiles.matrixMcpExtras')}</div>
              {#each r.mcp.extras as ex (ex)}
                <div class="mcprow">
                  <span class="min-w-0 flex-1 truncate font-mono text-sw-xs" title={ex}>{ex}</span>
                  <button
                    type="button"
                    class="xbtn"
                    disabled={busy}
                    onclick={() => onMcpRemoveExtra(ex, popFor!)}
                    title={t('profiles.matrixMcpRemoveTip')}
                  >✕</button>
                </div>
              {/each}
            {/if}
            {#if !missing.length && !r.mcp.extras.length}
              <div class="text-sw-xs text-sw-text-muted">{t('profiles.matrixMcpInSync')}</div>
            {/if}
          {/if}
        </div>
      {/if}
    {/if}

    <div class="applybar" class:has-changes={dirtyNames.length > 0}>
      {#if dirtyNames.length > 0}<span class="applybar-count">{t('profiles.matrixPending', { n: dirtyNames.length })}</span>{/if}
      <button class="sw-btn sw-btn-primary" disabled={!canApply} onclick={openPreview} title={t('profiles.matrixApplyTip')}>
        {applying ? t('profiles.matrixApplying') : t('profiles.matrixApply', { n: dirtyNames.length })}
      </button>
      <button class="sw-btn sw-btn-ghost" disabled={dirtyNames.length === 0 || applying} onclick={resetDraft} title={t('profiles.matrixResetTip')}>
        {t('profiles.matrixReset')}
      </button>
      <span class="text-sw-xs text-sw-text-muted">{t('profiles.matrixNoWrite')}</span>
    </div>
  {/if}
</section>

<ModalShell open={previewOpen} onClose={() => (previewOpen = false)} size="md">
  <h3 class="mb-sw-3 text-base font-semibold">{t('profiles.matrixPreviewTitle')}</h3>
  <div class="mb-sw-3 flex flex-col">
    {#each preview as c (c.who + c.cat)}
      <div class="chg">
        <span class="who">{c.who}</span>
        <span class="cat">{c.cat}</span>
        <span class="min-w-0 break-words">{c.text}</span>
      </div>
    {/each}
  </div>
  <div class="flex items-center justify-end gap-sw-2">
    <button class="sw-btn sw-btn-ghost" onclick={() => (previewOpen = false)}>{t('profiles.matrixPreviewBack')}</button>
    <button class="sw-btn sw-btn-primary" onclick={confirmApply}>{t('profiles.matrixPreviewConfirm')}</button>
  </div>
  <p class="mt-sw-2 text-sw-xs text-sw-text-muted">{t('profiles.matrixPreviewNote')}</p>
</ModalShell>

<style>
  .mx {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--sw-text-sm);
  }
  .mx th {
    text-align: left;
    padding: var(--sw-space-2) var(--sw-space-3);
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--sw-text-muted);
    border-bottom: 1px solid var(--sw-border);
    font-weight: 600;
  }
  .mx td {
    padding: var(--sw-space-2) var(--sw-space-3);
    border-bottom: 1px solid var(--sw-border);
    vertical-align: middle;
  }
  .mx tbody tr:last-child td {
    border-bottom: none;
  }
  .mx tr.dirty td {
    background: var(--sw-accent-glow);
  }
  .unsaved-pill {
    flex-shrink: 0;
    padding: 1px 7px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--sw-warn) 22%, transparent);
    color: var(--sw-warn);
    font-size: 10px;
    font-weight: 600;
    line-height: 1.4;
    white-space: nowrap;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 11px;
    border: 1px solid var(--sw-border);
    border-radius: 99px;
    background: var(--sw-bg-secondary);
    color: var(--sw-text-primary);
    font-size: var(--sw-text-xs);
    cursor: pointer;
  }
  .chip:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .chip.warn {
    border-color: var(--sw-warn);
  }
  .chip.dirtychip {
    border-color: var(--sw-accent);
  }
  .popsec {
    margin: 6px 0 2px;
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
  }
  .popsec:first-child {
    margin-top: 0;
  }
  .mcprow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
  }
  .xbtn {
    flex-shrink: 0;
    border: 1px solid var(--sw-border);
    border-radius: 6px;
    background: var(--sw-bg-secondary);
    color: var(--sw-warn);
    font-size: 11px;
    line-height: 1;
    padding: 3px 6px;
    cursor: pointer;
  }
  .xbtn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .mcpbtn {
    flex-shrink: 0;
    border: 1px solid var(--sw-accent);
    border-radius: 6px;
    background: var(--sw-bg-secondary);
    color: var(--sw-accent);
    font-size: var(--sw-text-xs);
    line-height: 1;
    padding: 4px 8px;
    cursor: pointer;
  }
  .mcpbtn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .warn {
    display: block;
    margin-top: 3px;
    color: var(--sw-warn);
    font-size: var(--sw-text-xs);
  }
  .popover {
    position: fixed;
    z-index: 60;
    min-width: 210px;
    padding: var(--sw-space-3);
    background: var(--sw-bg-secondary);
    border: 1px solid var(--sw-border);
    border-radius: var(--sw-radius-md);
    box-shadow: 0 12px 30px rgba(0, 0, 0, 0.35);
  }
  .poprow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 0;
    cursor: pointer;
  }
  .warnnote {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--sw-border);
    color: var(--sw-warn);
    font-size: var(--sw-text-xs);
  }
  .applybar {
    display: flex;
    align-items: center;
    gap: var(--sw-space-3);
    margin-top: var(--sw-space-4);
    /* Stick to the bottom of the viewport so pending edits + Apply stay reachable while scrolling
       the matrix — the accumulate-then-apply model was invisible when the bar sat far below. */
    position: sticky;
    bottom: 0;
    padding: var(--sw-space-3);
    border-radius: var(--sw-radius);
    /* z5_10: --sw-bg is undefined → the bar was transparent and table rows showed through it.
       Use the opaque surface token so the bar fully hides the rows it sticks over. */
    background: var(--sw-bg-primary);
    transition: background 0.15s ease, box-shadow 0.15s ease;
  }
  .applybar.has-changes {
    background: color-mix(in srgb, var(--sw-accent) 10%, var(--sw-bg-primary));
    box-shadow: 0 -2px 8px rgb(0 0 0 / 0.12), inset 0 0 0 1px var(--sw-accent);
  }
  .applybar-count {
    font-size: var(--sw-text-sm);
    font-weight: 600;
    color: var(--sw-accent-text);
  }
  .chg {
    display: flex;
    gap: var(--sw-space-2);
    align-items: baseline;
    padding: 6px 2px;
    border-bottom: 1px solid var(--sw-border);
    font-size: var(--sw-text-sm);
  }
  .chg:last-child {
    border-bottom: none;
  }
  .chg .who {
    min-width: 64px;
    font-weight: 600;
  }
  .chg .cat {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
    white-space: nowrap;
  }
</style>
