// Schedule tab state, moved verbatim out of +page.svelte following the EnvState/AgentsState pattern.
// Every action here is a streamed run, so the page hands its run lock in as `spawn` (the lock and
// the console log stay in +page); the confirm gate comes in the same way.
//
// Module singleton because ScheduleTab is remounted by the {#key active} tab chain — a per-instance
// state would re-spawn the (pwsh-spawning) query on every visit, whereas the page's old
// `schedulesLoaded` flag loaded once per session. The `loaded` flag keeps that contract.
import type { SchedulesStatus, ScheduleAction } from '$lib/ipc';
import { readSchedules, readSchedulesCached, runSchedule } from '$lib/ipc';
import { t } from '$lib/i18n';
import type { ConfirmRequest } from '$lib/confirmGate';
import type { SpawnRun } from '$lib/runSpawn';

export class SchedState {
  data = $state<SchedulesStatus | null>(null);
  /** First-open gate (was `schedulesLoaded` in +page) — survives the tab's remount. */
  loaded = $state(false);
  /** Drives the page's refresh overlay + sidebar spinner for this tab. */
  loading = $state(false);
  /** $lib/confirmGate is the ONE confirm mechanism; +page hands its gate in. */
  askConfirm: (req: ConfirmRequest) => void = () => {};
  /** The global run lock lives in +page; this is its entry point. */
  spawn: SpawnRun = () => false;

  async load() {
    try {
      this.data = await readSchedules();
    } catch {
      this.data = null;
    }
  }
  /** Lazy first-open load — read_schedules spawns pwsh, so it must not run before the tab opens. */
  loadOnce() {
    // L54: set the flag SYNCHRONOUSLY — a quick tab toggle during the (slow, spawning) load
    // would otherwise re-fire this while the first IPC is still in flight.
    if (this.loaded) return;
    this.loaded = true;
    this.loading = true;
    void this.load().finally(() => (this.loading = false));
  }
  /** Seed from the on-disk envelope (file-only, no pwsh) so Home's task rollup shows from launch. */
  async seedCached() {
    try {
      const s = await readSchedulesCached();
      if (s && this.data == null) this.data = s;
    } catch {
      /* ignore — the tab's own lazy read is the authoritative one */
    }
  }

  private start(action: ScheduleAction, id: string, time?: string) {
    const verb: Record<ScheduleAction, string> = {
      enable: t('page.sched_verb_enable'),
      disable: t('page.sched_verb_disable'),
      run: t('page.sched_verb_run'),
      create: t('page.sched_verb_create'),
      delete: t('page.sched_verb_delete')
    };
    // busyToast: don't silently drop the click — tell the user why (a run holds the single slot).
    // Without it, clicking "Создать расписание" while busy looked like a dead button.
    this.spawn('schedule', t('page.sched_log', { id, verb: verb[action] }), () => runSchedule(action, id, time), {
      busyToast: true
    });
  }

  action(action: ScheduleAction, id: string, time?: string) {
    if (action === 'delete') {
      this.askConfirm({
        title: t('page.confirm_sched_delete_title'),
        message: t('page.confirm_sched_delete_msg', { id }),
        confirmLabel: t('page.confirm_sched_delete_btn'),
        action: () => this.start('delete', id)
      });
    } else {
      this.start(action, id, time);
    }
  }
}

export const schedState = new SchedState();
