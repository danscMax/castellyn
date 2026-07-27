// Backup tab state, moved verbatim out of +page.svelte following the EnvState/AgentsState pattern.
// Every action here is a streamed run, so the page hands its run lock in as `spawn` (the lock and
// the console log stay in +page); the confirm gate comes in the same way.
//
// Module singleton, and NOT lazily loaded: `data` feeds the sidebar attention badge (backup +
// home), so +page loads it on mount regardless of which tab is open.
import type { BackupList, BackupAction, RestoreOpts } from '$lib/ipc';
import { listBackups, runBackup } from '$lib/ipc';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';
import type { SpawnRun } from '$lib/runSpawn';

export class BackupState {
  data = $state<BackupList | null>(null);
  /** $lib/confirmGate is the ONE confirm mechanism; +page hands its gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};
  /** The global run lock lives in +page; this is its entry point. */
  spawn: SpawnRun = () => false;
  /**
   * A restore that was actually started, so +page's run-done can offer "undo" (re-restore the
   * previous snapshot). Armed ONLY when the spawn was accepted — a stale value would later attach
   * a real destructive restore to an unrelated backup run. Consumed (and nulled) by run-done.
   */
  pendingUndo: { snapshot: string; profiles?: string[]; includeCredentials?: boolean } | null = null;

  async load() {
    try {
      this.data = await listBackups();
    } catch {
      this.data = null;
    }
  }

  /** Returns false when the global run lock rejected the spawn — nothing was started. */
  start(action: BackupAction, opts?: RestoreOpts): boolean {
    const verb =
      action === 'backup'
        ? t('page.backup_verb_snapshot')
        : action === 'restore-preview'
          ? t('page.backup_verb_restore_preview')
          : action === 'delete-snapshot'
            ? t('page.backup_verb_delete')
            : t('page.backup_verb_restore');
    return this.spawn('backup', t('page.backup_log', { verb }), () => runBackup(action, opts));
  }

  action(action: BackupAction, opts?: RestoreOpts) {
    if (action === 'restore') {
      const snap = opts?.timestamp ?? t('page.backup_snap_last');
      this.askConfirm({
        title: t('page.confirm_restore_title'),
        message: t('page.confirm_restore_msg', { snap }),
        confirmLabel: t('page.confirm_restore_btn'),
        action: () => {
          // Arm the undo ONLY if the spawn was accepted: start() no-ops while the global run lock
          // is held, and a stale pendingUndo would later attach an "undo the restore" toast —
          // whose action launches a real destructive restore — to an unrelated backup run.
          const started = this.start('restore', opts);
          this.pendingUndo =
            started && opts
              ? { snapshot: opts.timestamp ?? '', profiles: opts.profiles, includeCredentials: opts.includeCredentials }
              : null;
        },
        danger: true,
        requireText: opts?.timestamp ?? null
      });
    } else if (action === 'delete-snapshot') {
      this.askConfirm({
        title: t('page.confirm_delsnap_title'),
        message: t('page.confirm_delsnap_msg', { snap: opts?.timestamp ?? '' }),
        confirmLabel: t('page.confirm_delsnap_btn'),
        action: () => this.start('delete-snapshot', opts),
        danger: true
      });
    } else {
      this.start(action, opts);
    }
  }
}

export const backupState = new BackupState();
