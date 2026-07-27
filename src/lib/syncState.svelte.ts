// Sync tab state (Syncthing status + config drift), moved verbatim out of +page.svelte following the
// EnvState/AgentsState pattern. Both of its runners are streamed runs, so the page hands its run
// lock in as `spawn` (the lock and the console log stay in +page), the confirm gate in as
// `askConfirm`, and — because the first-open load also refreshes profiles (the tab renders the sync
// CONFLICT list, which lives in the profiles envelope) — `reloadProfiles` too.
//
// Module singleton because SyncTab is remounted by the {#key active} tab chain; the `loaded` flag
// keeps the page's old load-once-per-session `syncLoaded` contract.
import type { SyncStatus, ConfigDriftStatus, ConfigDriftAction } from '$lib/ipc';
import { readSync, runSync, readConfigDrift, runConfigDrift } from '$lib/ipc';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';
import type { SpawnRun } from '$lib/runSpawn';

const ALL_ITEMS = ['history', 'projects', 'skills', 'agents', 'commands', 'keybindings', 'castellyn'];

export class SyncState {
  data = $state<SyncStatus | null>(null);
  drift = $state<ConfigDriftStatus | null>(null);
  /** First-open gate (was `syncLoaded` in +page) — survives the tab's remount. */
  loaded = $state(false);
  /** Drives the page's refresh overlay + sidebar spinner for this tab. */
  loading = $state(false);
  /** $lib/confirmGate is the ONE confirm mechanism; +page hands its gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};
  /** The global run lock lives in +page; this is its entry point. */
  spawn: SpawnRun = () => false;
  /** Profiles data is +page's; this tab reads its conflict list, so it refreshes it on first open. */
  reloadProfiles: () => Promise<void> = async () => {};

  async load() {
    try {
      this.data = await readSync();
    } catch {
      this.data = null;
    }
  }
  async loadDrift() {
    try {
      this.drift = await readConfigDrift();
    } catch {
      this.drift = null;
    }
  }
  /** Lazy-load on first open + run a fresh query to fetch live Syncthing status. */
  loadOnce() {
    if (this.loaded) return;
    this.loaded = true;
    this.loading = true;
    void Promise.all([this.load(), this.loadDrift(), this.reloadProfiles()])
      .then(() => this.query())
      .finally(() => (this.loading = false));
  }

  // run_sync is a direct async command (no spawn_streamed → no run-done event), so the promise
  // resolving IS the completion signal — hence `awaited`, which releases the lock. Otherwise
  // `running='sync'` sticks forever and `busy` disables controls across every tab (the
  // "everything is dead" symptom).
  private start(action: 'query' | 'set', enabled?: string[]) {
    this.spawn(
      'sync',
      action === 'set' ? t('page.sync_log_set') : t('page.sync_log_query'),
      // Re-read the status after the run so the UI reflects reality — otherwise after "Применить"
      // (set) the "требует применения" banner never clears (stignoreMatches stays false) and the
      // button just re-appears, reading as a no-op. load() is self-guarded (never throws).
      () => runSync(action, enabled).then(() => this.load()),
      { awaited: true }
    );
  }

  query() {
    this.start('query');
  }

  apply(enabled: string[]) {
    const off = ALL_ITEMS.filter((i) => !enabled.includes(i));
    // Enabling/keeping sync items is additive and safe — only gate behind a confirm when something
    // is actually being DISABLED (the destructive direction).
    if (!off.length) {
      this.start('set', enabled);
      return;
    }
    this.askConfirm({
      title: t('page.confirm_sync_title'),
      message: t('page.sync_apply_off', { off: off.join(', ') }),
      confirmLabel: t('page.confirm_sync_btn'),
      action: () => this.start('set', enabled)
    });
  }

  // --- Config drift (FUN-7): shares the 'sync' run slot + outcome/toast ---
  private startDrift(action: ConfigDriftAction) {
    const verb =
      action === 'relink'
        ? t('page.drift_verb_relink')
        : action === 'sync-now'
          ? t('page.drift_verb_sync')
          : t('page.drift_verb_check');
    // Re-read drift + sync state after the run (relink/sync-now/check) so the UI reflects the result
    // — without this "Починить связи" finished (exit 0) but the drift status never refreshed, so the
    // banner looked unchanged ("did it work?"). `awaited` also clears the 'sync' lock
    // (run_config_drift is a direct invoke with no run-done, so nothing else releases it → the whole
    // UI stayed busy).
    this.spawn(
      'sync',
      t('page.drift_log', { verb }),
      () => runConfigDrift(action).then(() => Promise.all([this.loadDrift(), this.load()])),
      { awaited: true }
    );
  }

  driftAction(action: ConfigDriftAction) {
    if (action === 'check') {
      this.startDrift('check');
    } else if (action === 'relink') {
      this.askConfirm({
        title: t('page.confirm_relink_title'),
        message: t('page.confirm_relink_msg'),
        confirmLabel: t('page.confirm_relink_btn'),
        action: () => this.startDrift('relink')
      });
    } else {
      this.askConfirm({
        title: t('page.confirm_driftsync_title'),
        message: t('page.confirm_driftsync_msg'),
        confirmLabel: t('page.confirm_driftsync_btn'),
        action: () => this.startDrift('sync-now')
      });
    }
  }
}

export const syncState = new SyncState();
