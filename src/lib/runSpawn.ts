// The signature of +page.svelte's `spawnRun` — the ONE way a tab state class starts a streamed run.
//
// The run lock (`running`) and the console buffer (`log`) stay in +page; a state class never owns
// either. It receives this function the same way it receives the confirm gate, so the lock has
// exactly one owner and one entry point.
//
// Returns false when the lock rejected the spawn (something else is running) — nothing was started.
export type SpawnRun = (
  id: string,
  line: string,
  run: () => Promise<unknown>,
  opts?: {
    /** The IPC promise IS the completion signal (no run-done event) → release the lock on settle. */
    awaited?: boolean;
    /** Surface an info toast naming the in-flight run instead of dropping the click silently. */
    busyToast?: boolean;
  }
) => boolean;
