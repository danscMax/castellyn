// Plugins & Skills ("extensions") tab state, moved verbatim out of +page.svelte following the
// EnvState/AgentsState pattern.
//
// Most actions here are streamed runs, so the page hands its run lock in as `spawn` (the lock and
// the console log stay in +page); the confirm gate comes in the same way. The bulk sweep is the
// exception — it runs in its OWN backend domain and deliberately does NOT take the global lock, so
// it writes the console through the injected `resetLog`/`appendLog` instead of through `spawn`.
//
// Module singleton because PluginsTab is remounted when +page first opens the tab; the `loaded`
// flag keeps the page's old load-once-per-session `extensionsLoaded` contract.
import type {
  PluginInfo,
  SkillInfo,
  PluginAction,
  PluginUpdate,
  PluginContents,
  PluginSyncStatus,
  BumpLevel
} from '$lib/ipc';
import {
  listPlugins,
  listSkills,
  deleteSkill,
  listPluginUpdates,
  listPluginContents,
  runPlugin,
  runPluginsBulk,
  runMarketplaceBump,
  pluginSyncStatus,
  pluginSyncSet,
  runPluginSync,
  unblockManagedPlugin,
  runManagedDeploy,
  canonicalSkillsDir,
  openPath
} from '$lib/ipc';
import { pushToast } from '$lib/toast.svelte';
import { opName } from '$lib/running.svelte';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';
import type { SpawnRun } from '$lib/runSpawn';

export class PluginsState {
  plugins = $state<PluginInfo[] | null>(null);
  skills = $state<SkillInfo[] | null>(null);
  updates = $state<PluginUpdate[]>([]);
  contents = $state<PluginContents[]>([]);
  syncStatus = $state<PluginSyncStatus | null>(null);
  /**
   * True while a bulk plugin op is in flight. F17: the bulk run lives in its OWN backend domain
   * (run_plugins_bulk), so it does NOT take the global `running` lock — only plugin-tab buttons are
   * gated, leaving backup/forks/etc. usable. +page's run-done listener still skips plugin-mgr while
   * this is set (the await drives completion, not the event), and "cancel all" clears it.
   */
  bulkActive = $state(false);
  /** First-open gate (was `extensionsLoaded` in +page) — survives the tab's remount. */
  loaded = $state(false);
  /** Drives the page's refresh overlay + sidebar spinner for this tab. */
  loading = $state(false);
  /** $lib/confirmGate is the ONE confirm mechanism; +page hands its gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};
  /** The global run lock lives in +page; this is its entry point. */
  spawn: SpawnRun = () => false;
  /** +page's toastErr — logs the failure to the console AND toasts it. */
  toastErr: (e: unknown, title?: string) => void = () => {};
  /** +page owns the console buffer: replace it with one line / append one line. */
  resetLog: (line: string) => void = () => {};
  appendLog: (line: string) => void = () => {};
  /** Stack-ownership drift is +page's (Home card + sidebar badge); unblocking a plugin changes it. */
  reloadStackDrift: () => Promise<void> = async () => {};

  async loadUpdates() {
    try {
      this.updates = await listPluginUpdates();
    } catch {
      this.updates = [];
    }
  }
  // list_plugin_updates spawns the claude CLI — throttle the on-focus refresh so alt-tabbing
  // doesn't fire a spawn every time the window regains focus (explicit calls stay unthrottled).
  private lastFocusCheck = 0;
  loadUpdatesOnFocus() {
    const now = Date.now();
    if (now - this.lastFocusCheck > 5 * 60_000) {
      this.lastFocusCheck = now;
      void this.loadUpdates();
    }
  }

  async load() {
    // Load plugins + skills INDEPENDENTLY: list_plugins spawns the (slow, sometimes-hanging) claude
    // CLI and must not gate the local skill scan. On failure set [] (not null) and surface the real
    // error — a null rendered as an eternal skeleton, indistinguishable from loading, was the
    // "infinite loading" bug (the empty-state already hints "or claude CLI unavailable").
    const pP = listPlugins()
      .then((v) => {
        this.plugins = v;
      })
      .catch((e) => {
        this.plugins = [];
        // U8/U3: a localized title + the raw error in `detail` (was: the raw serde string dumped
        // straight into a Russian-UI toast title). The empty-state already explains "no plugins".
        this.toastErr(e, t('page.plugins_load_error'));
      });
    const pS = listSkills()
      .then((v) => {
        this.skills = v;
      })
      .catch(() => {
        this.skills = [];
      });
    await Promise.allSettled([pP, pS]);
    try {
      this.contents = await listPluginContents();
    } catch {
      this.contents = [];
    }
    try {
      this.syncStatus = await pluginSyncStatus();
    } catch {
      this.syncStatus = null;
    }
    await this.loadUpdates();
  }

  /** Lazy first-open load — list_plugins spawns the claude CLI, so it must not run before the tab opens. */
  loadOnce() {
    // L54: set the flag SYNCHRONOUSLY — a tab toggle mid-flight used to fire this loader twice.
    if (this.loaded) return;
    this.loaded = true;
    this.loading = true;
    void this.load().finally(() => (this.loading = false));
  }

  private start(action: PluginAction, id: string) {
    const verb =
      action === 'update'
        ? t('page.plugin_verb_update')
        : action === 'enable'
          ? t('page.plugin_verb_enable')
          : action === 'remove'
            ? t('page.plugin_verb_remove')
            : t('page.plugin_verb_disable');
    this.spawn('plugin-mgr', t('page.plugin_log', { id, verb }), () => runPlugin(action, id));
  }

  action(action: PluginAction, id: string) {
    // 'disable' is reversible (re-enable any time) → no confirm, matching bulk-disable. Only the
    // irreversible 'remove' gates behind a danger confirm.
    if (action === 'remove') {
      this.askConfirm({
        title: t('page.confirm_plugin_remove_title'),
        message: t('page.confirm_plugin_remove_msg', { id }),
        confirmLabel: t('page.confirm_plugin_remove_btn'),
        action: () => this.start('remove', id),
        danger: true
      });
    } else {
      this.start(action, id);
    }
  }

  // Ф3: own-marketplace version bump — same single-run contract as start() (component
  // "plugin-mgr"), run-done releases the lock and toasts via the generic operational path.
  bump(id: string, level: BumpLevel) {
    this.spawn('plugin-mgr', t('page.plugin_bump_log', { id, level }), () => runMarketplaceBump(id, level));
  }

  // A managed-policy-blocked plugin can't be enabled per-profile — the real fix is removing its
  // explicit `false` from the SOURCE managed-settings.json and redeploying (one UAC prompt).
  unblock(id: string) {
    this.askConfirm({
      title: t('page.confirm_unblock_title'),
      message: t('page.confirm_unblock_msg', { id }),
      confirmLabel: t('page.confirm_unblock_btn'),
      action: async () => {
        try {
          await unblockManagedPlugin(id);
          const res = await runManagedDeploy();
          if (res.state === 'ok') {
            // Unblocking is only half the intent — the plugin is still disabled per-profile.
            // Offer the enable right on the toast instead of demanding another hunt+click.
            pushToast({
              kind: 'success',
              title: t('page.unblock_done', { id }),
              action: { label: t('page.unblock_enable_now'), onClick: () => this.start('enable', id) }
            });
          } else {
            pushToast({ kind: 'error', title: t('page.toast_generic_error'), detail: res.detail });
          }
          await Promise.all([this.load(), this.reloadStackDrift()]);
        } catch (e) {
          this.toastErr(e);
        }
      }
    });
  }

  // F17: one backend call runs the whole bulk in its own domain (sequential there — no config race),
  // streaming id-tagged lines to the run log. We don't set the global `running` lock, so unrelated
  // work stays available; only the plugin tab is gated via `bulkActive`.
  private async runBulk(action: PluginAction, ids: string[]) {
    if (!ids.length || this.bulkActive) return;
    const verb =
      action === 'update'
        ? t('page.plugin_verb_update')
        : action === 'enable'
          ? t('page.plugin_verb_enable')
          : action === 'remove'
            ? t('page.plugin_verb_remove')
            : t('page.plugin_verb_disable');
    this.bulkActive = true;
    this.resetLog(t('page.plugin_bulk_log', { n: ids.length, verb }));
    try {
      await runPluginsBulk(action, ids);
    } catch (e) {
      this.appendLog(t('page.log_error', { e: String(e) }));
    } finally {
      this.bulkActive = false;
      this.load();
    }
  }

  bulkAction(action: PluginAction, ids: string[]) {
    if (!ids.length) return;
    if (action === 'remove') {
      this.askConfirm({
        title: t('page.confirm_plugin_remove_title'),
        message: t('page.confirm_plugin_bulk_remove_msg', { count: ids.length }),
        confirmLabel: t('page.confirm_plugin_remove_btn'),
        action: () => this.runBulk('remove', ids),
        danger: true,
        details: ids
      });
    } else {
      this.runBulk(action, ids);
    }
  }

  // Cross-profile plugin sync: one-off reconcile (streams into the console, run-done
  // releases the lock + toasts via the generic operational path).
  syncNow() {
    this.spawn(
      'pluginsync',
      t('page.log_component', { name: opName('pluginsync'), verb: t('page.verb_apply') }),
      () => runPluginSync()
    );
  }

  // Wire/unwire the SessionStart auto-sync hook in every profile (quick file edits, no run lock).
  async syncHookToggle(enabled: boolean) {
    try {
      this.syncStatus = await pluginSyncSet(enabled);
      pushToast({ kind: 'success', title: t(enabled ? 'page.pluginsync_hook_on' : 'page.pluginsync_hook_off') });
    } catch (e) {
      this.toastErr(e);
    }
  }

  openSkills() {
    canonicalSkillsDir()
      .then((d) => openPath(d))
      .catch((e) => this.toastErr(e));
  }
  openSkill(dir: string) {
    openPath(dir).catch((e) => this.toastErr(e));
  }
  // Named removeSkill, not deleteSkill: the imported IPC `deleteSkill` is called inside it.
  removeSkill(dir: string, name: string) {
    this.askConfirm({
      title: t('page.confirm_skill_delete_title'),
      message: t('page.confirm_skill_delete_msg', { name }),
      confirmLabel: t('page.confirm_skill_delete_btn'),
      action: () => {
        deleteSkill(dir)
          .then(() => this.load())
          .catch((e) => this.toastErr(e));
      },
      danger: true
    });
  }
}

export const pluginsState = new PluginsState();
