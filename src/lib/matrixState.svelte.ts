// B3: shared per-profile matrix editing state, extracted verbatim from ProfilesMatrix.svelte so the
// same accumulate → preview → apply model can drive the profile table's expandable row editors
// (MatrixRowEditor) and the single controls bar (MatrixControls). Behaviour is unchanged from the
// old standalone matrix — only the rendering location moved (Profiles tab, one table).
import type { EngineStatus, MyProvider, MatrixRow, MatrixApply } from '$lib/ipc';
import { readProfileMatrix } from '$lib/ipc';
import { t } from '$lib/i18n';
import { isValidHttpUrl, urlHost } from '$lib/url';

// plugins = per-id explicit override (true/false); absent id = no draft edit (baseline stands).
type Draft = { provider: string; proxy: string; folders: string[]; plugins: Record<string, boolean> };
export type PopKind = 'folders' | 'plugins' | 'mcp';

export class MatrixState {
  // --- Inputs (kept in sync by the Profiles tab) --------------------------------------------
  engines = $state<EngineStatus[] | null>([]);
  myProviders = $state<MyProvider[] | null>([]);
  running = $state<string | null>(null);
  onApplyMatrix: (changes: MatrixApply) => Promise<{ skipped: string[] }> = async () => ({ skipped: [] });
  onMcpDeployProfile: (profile: string) => void = () => {};
  onMcpRemoveExtra: (server: string, profile: string) => void = () => {};

  // --- Baseline (server truth) + local draft overlay ----------------------------------------
  rows = $state<MatrixRow[]>([]);
  loaded = $state(false);
  loadErr = $state('');
  draft = $state<Record<string, Draft>>({});
  applying = $state(false);

  // --- Popover (folders / plugins / mcp — one at a time, anchored to the clicked chip) -------
  popFor = $state<string | null>(null);
  popKind = $state<PopKind>('folders');
  popAnchor = $state<HTMLElement | null>(null);
  previewOpen = $state(false);

  busy = $derived(!!this.running);

  // '' base URL = OAuth/subscription (no custom provider). Draft mirrors that convention.
  private seedRow(r: MatrixRow): Draft {
    return {
      provider: r.provider.baseUrl ?? '',
      proxy: r.proxy ?? '',
      folders: r.folders.filter((f) => f.desired).map((f) => f.name),
      plugins: {}
    };
  }
  private seed(list: MatrixRow[]) {
    const d: Record<string, Draft> = {};
    for (const r of list) d[r.name] = this.seedRow(r);
    this.draft = d;
  }
  // reseed=false: refresh server truth but keep the current draft (used for the mcpTick re-read,
  // where only mcp facts changed and the user may have unsaved provider/plugin edits to preserve).
  async load(reseed = true) {
    try {
      const list = await readProfileMatrix();
      this.rows = list;
      if (reseed) this.seed(list);
      else for (const r of list) if (!this.draft[r.name]) this.draft[r.name] = this.seedRow(r);
      this.loadErr = '';
    } catch (e) {
      this.loadErr = String(e);
    } finally {
      this.loaded = true;
    }
  }

  // --- Provider options (anthropic engines + saved custom providers + OAuth) -----------------
  providerOptions = $derived([
    { value: '', label: t('profiles.matrixProviderOauth') },
    ...(this.engines ?? [])
      .filter((e) => e.protocol === 'anthropic' && e.baseUrl)
      .map((e) => ({ value: e.baseUrl, label: e.name })),
    ...(this.myProviders ?? [])
      .filter((m) => m.baseUrl && !(this.engines ?? []).some((e) => e.baseUrl === m.baseUrl))
      .map((m) => ({ value: m.baseUrl, label: m.name }))
  ]);
  providerLabel(baseUrl: string): string {
    if (!baseUrl) return t('profiles.matrixProviderOauth');
    const e = (this.engines ?? []).find((x) => x.baseUrl === baseUrl);
    if (e) return e.name;
    const m = (this.myProviders ?? []).find((x) => x.baseUrl === baseUrl);
    if (m) return m.name;
    return urlHost(baseUrl);
  }
  // Model fields for a chosen provider come from a saved custom provider; local engines carry none.
  private modelFor(baseUrl: string): { model: string; smallModel: string } {
    const m = (this.myProviders ?? []).find((x) => x.baseUrl === baseUrl);
    return { model: m?.model ?? '', smallModel: m?.smallModel ?? '' };
  }

  // --- Dirty tracking ------------------------------------------------------------------------
  rowByName = $derived(new Map(this.rows.map((r) => [r.name, r])));
  private baseFolders(r: MatrixRow): string[] {
    return r.folders.filter((f) => f.desired).map((f) => f.name);
  }
  private eqSet(a: string[], b: string[]): boolean {
    if (a.length !== b.length) return false;
    const s = new Set(a);
    return b.every((x) => s.has(x));
  }
  providerChanged(name: string): boolean {
    const r = this.rowByName.get(name);
    return !!r && (this.draft[name]?.provider ?? '') !== (r.provider.baseUrl ?? '');
  }
  proxyChanged(name: string): boolean {
    const r = this.rowByName.get(name);
    return !!r && (this.draft[name]?.proxy ?? '').trim() !== (r.proxy ?? '');
  }
  foldersChanged(name: string): boolean {
    const r = this.rowByName.get(name);
    return !!r && !this.eqSet(this.draft[name]?.folders ?? [], this.baseFolders(r));
  }
  // --- Plugins (per-profile enabledPlugins override) -----------------------------------------
  pluginOn(name: string, p: { id: string; state: string }): boolean {
    const ov = this.draft[name]?.plugins[p.id];
    return ov === undefined ? p.state === 'on' : ov;
  }
  pluginDirty(name: string, p: { id: string; state: string }): boolean {
    const ov = this.draft[name]?.plugins[p.id];
    if (ov === undefined) return false;
    if (p.state === 'on') return ov === false;
    if (p.state === 'off') return ov === true;
    return true; // unset
  }
  pluginsChanged(name: string): boolean {
    const r = this.rowByName.get(name);
    return !!r && r.plugins.some((p) => this.pluginDirty(name, p));
  }
  togglePlugin(name: string, id: string, on: boolean) {
    this.draft[name] = { ...this.draft[name], plugins: { ...this.draft[name].plugins, [id]: on } };
  }
  pluginOnCount(name: string): number {
    const r = this.rowByName.get(name);
    return r ? r.plugins.filter((p) => this.pluginOn(name, p)).length : 0;
  }
  // short label: strip any @version tail.
  pluginShort = (id: string): string => id.split('@')[0];

  // --- MCP facts (read-only reconcile status) ------------------------------------------------
  mcpMissing(r: MatrixRow): string[] {
    const have = new Set(r.mcp.deployed);
    return r.mcp.canon.filter((c) => !have.has(c));
  }
  mcpWarn(r: MatrixRow): boolean {
    return this.mcpMissing(r).length > 0 || r.mcp.extras.length > 0;
  }

  rowDirty(name: string): boolean {
    return (
      this.providerChanged(name) ||
      this.proxyChanged(name) ||
      this.foldersChanged(name) ||
      this.pluginsChanged(name)
    );
  }
  dirtyNames = $derived(this.rows.map((r) => r.name).filter((n) => this.rowDirty(n)));
  // A proxy edit that isn't a clear must be a valid http(s)/socks URL, else Apply is blocked.
  proxyValid(name: string): boolean {
    const v = (this.draft[name]?.proxy ?? '').trim();
    return v === '' || isValidHttpUrl(v) || /^socks(4a?|5h?):\/\//i.test(v);
  }
  anyInvalid = $derived(this.dirtyNames.some((n) => !this.proxyValid(n)));
  canApply = $derived(this.dirtyNames.length > 0 && !this.anyInvalid && !this.busy && !this.applying);

  togglePop(name: string, kind: PopKind, el: HTMLElement) {
    if (this.popFor === name && this.popKind === kind) {
      this.popFor = null;
      return;
    }
    this.popAnchor = el;
    this.popKind = kind;
    this.popFor = name;
  }
  toggleFolder(name: string, folder: string, on: boolean) {
    const cur = this.draft[name].folders;
    this.draft[name] = {
      ...this.draft[name],
      folders: on ? [...new Set([...cur, folder])] : cur.filter((f) => f !== folder)
    };
  }
  // Amber the chip when the profile isn't fully linked (needs attention / relink).
  folderWarn(r: MatrixRow, name: string): boolean {
    const sel = this.draft[name]?.folders ?? [];
    if (sel.length < r.folders.length) return true;
    return r.folders.some((f) => f.desired && f.actual !== 'linked');
  }

  // --- Preview + apply -----------------------------------------------------------------------
  preview = $derived.by<{ who: string; cat: string; text: string }[]>(() => {
    const out: { who: string; cat: string; text: string }[] = [];
    for (const r of this.rows) {
      const d = this.draft[r.name];
      if (!d) continue;
      if (this.providerChanged(r.name)) {
        out.push({
          who: r.name,
          cat: t('profiles.matrixCatProvider'),
          text: `${this.providerLabel(r.provider.baseUrl ?? '')} → ${this.providerLabel(d.provider)}`
        });
      }
      if (this.proxyChanged(r.name)) {
        const from = r.proxy || t('profiles.matrixProxyNone');
        const to = d.proxy.trim() || t('profiles.matrixProxyNone');
        out.push({ who: r.name, cat: t('profiles.matrixCatProxy'), text: `${from} → ${to}` });
      }
      if (this.foldersChanged(r.name)) {
        const before = new Set(this.baseFolders(r));
        const after = new Set(d.folders);
        const added = [...after].filter((f) => !before.has(f));
        const removed = [...before].filter((f) => !after.has(f));
        const parts = [...removed.map((f) => `−${f}`), ...added.map((f) => `+${f}`)];
        out.push({ who: r.name, cat: t('profiles.matrixCatFolders'), text: parts.join(', ') });
      }
      if (this.pluginsChanged(r.name)) {
        const on = r.plugins.filter((p) => this.pluginDirty(r.name, p) && d.plugins[p.id] === true);
        const off = r.plugins.filter((p) => this.pluginDirty(r.name, p) && d.plugins[p.id] === false);
        const parts = [...off.map((p) => `−${this.pluginShort(p.id)}`), ...on.map((p) => `+${this.pluginShort(p.id)}`)];
        out.push({ who: r.name, cat: t('profiles.matrixCatPlugins'), text: parts.join(', ') });
      }
    }
    return out;
  });

  private buildChanges(): MatrixApply {
    const providers: MatrixApply['providers'] = [];
    const proxies: MatrixApply['proxies'] = [];
    const folders: MatrixApply['folders'] = [];
    const plugins: MatrixApply['plugins'] = [];
    for (const r of this.rows) {
      const d = this.draft[r.name];
      if (!d) continue;
      if (this.providerChanged(r.name)) {
        const base = d.provider || null;
        const m = base ? this.modelFor(base) : { model: '', smallModel: '' };
        providers.push({ name: r.name, baseUrl: base, model: m.model, smallModel: m.smallModel });
      }
      if (this.proxyChanged(r.name)) proxies.push({ name: r.name, url: d.proxy.trim() });
      if (this.foldersChanged(r.name)) folders.push({ name: r.name, folders: d.folders });
      if (this.pluginsChanged(r.name)) {
        const enable = r.plugins.filter((p) => this.pluginDirty(r.name, p) && d.plugins[p.id] === true).map((p) => p.id);
        const disable = r.plugins.filter((p) => this.pluginDirty(r.name, p) && d.plugins[p.id] === false).map((p) => p.id);
        plugins.push({ name: r.name, enable, disable });
      }
    }
    return { providers, proxies, folders, plugins };
  }

  openPreview() {
    if (!this.canApply) return;
    this.previewOpen = true;
  }
  resetDraft() {
    this.seed(this.rows);
  }
  async confirmApply() {
    this.previewOpen = false;
    this.applying = true;
    let ok = true;
    try {
      await this.onApplyMatrix(this.buildChanges());
    } catch {
      ok = false; // +page surfaces the error toast
    } finally {
      // Re-read to verify actual state. Success → reseed (draft = baseline). Failure → keep the
      // draft overlay: applied rows auto-clear (baseline now matches), failed rows stay dirty.
      await this.load(ok);
      this.applying = false;
    }
  }
}
