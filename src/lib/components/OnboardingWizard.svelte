<script lang="ts">
  // First-run onboarding wizard (OU-04). A short multi-step modal, shown once when a fresh user
  // has neither a configured Scripts root nor any profiles, that walks them through the minimum
  // setup before they land on an empty Updates tab. Reuses ModalShell (focus trap / Esc), the
  // FolderField picker, and the same SettingsTab / Profiles IPC — no new backend.
  import ModalShell from './ModalShell.svelte';
  import FolderField from './FolderField.svelte';
  import { t } from '$lib/i18n';
  import { readConfig, saveConfig, appPaths, scriptsAvailable } from '$lib/ipc';
  import type { ProfileMgmtArgs } from '$lib/ipc';

  let {
    open = false,
    profileCount = 0,
    busy = false,
    onCreateProfile,
    onOpenProfiles,
    onFinish
  }: {
    open?: boolean;
    /** How many profiles currently exist (drives step 3's state + Next gating). */
    profileCount?: number;
    /** A profile-create run is in flight (mirrors the global run lock). */
    busy?: boolean;
    /** Create a profile inline via the existing run_profile_mgmt flow. */
    onCreateProfile: (args: ProfileMgmtArgs) => void;
    /** Switch to the Profiles tab (used when inline-create isn't wanted). */
    onOpenProfiles: () => void;
    /** Dismiss the wizard; runCheck = true → kick off a first "check all". */
    onFinish: (runCheck: boolean) => void;
  } = $props();

  const TOTAL = 4;
  let stepIdx = $state(0); // 0..3

  // Step 2 — Scripts root. Seed from the live config on open; persist via the same path SettingsTab uses.
  let scriptsRoot = $state('');
  let scriptsSaved = $state(false);
  // OSS first-run: null = unknown, false = maintenance scripts absent (or user said "I have none") →
  // profile-create + "check all" can't run, so those actions render disabled with a hint.
  let scriptsPresent = $state<boolean | null>(null);
  let seeded = false;
  $effect(() => {
    if (open && !seeded) {
      seeded = true;
      readConfig()
        .then((c) => {
          if (c.scriptsRoot) scriptsRoot = c.scriptsRoot;
        })
        .catch(() => {});
      scriptsAvailable()
        .then((v) => (scriptsPresent = v))
        .catch(() => {});
    }
    if (!open) {
      seeded = false;
      scriptsPresent = null;
      // Reset step + stale save-error so reopening the wizard starts fresh instead of resuming
      // on the last step with an old error banner.
      stepIdx = 0;
      saveErr = '';
    }
  });
  // The scripts step's "I don't have these scripts" escape: skip ahead and remember the absence so
  // the profile + check-all actions downstream disable themselves.
  function skipScripts() {
    scriptsPresent = false;
    if (stepIdx < TOTAL - 1) stepIdx += 1;
  }
  // A typed/picked path is "unsaved" until persisted — re-arm the save hint when it changes.
  $effect(() => {
    void scriptsRoot;
    scriptsSaved = false;
    saveErr = '';
  });

  let saveErr = $state('');
  async function saveScripts() {
    const root = scriptsRoot.trim();
    if (!root) return;
    await saveConfig((c) => (c.scriptsRoot = root)); // R7: rev-safe write
    await appPaths().catch(() => {}); // nudge the backend to resolve the new root
    scriptsSaved = true;
  }

  // Step 3 — create a profile inline (same validation/flow as ProfilesTab's add dialog).
  let profName = $state('');
  const nameValid = $derived(/^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/.test(profName));
  function createProfile() {
    if (!nameValid) return;
    onCreateProfile({ action: 'add', name: profName.trim(), color: 'White' });
    profName = '';
  }

  // Next is gated on the current step's requirement; the welcome + finish steps are always passable.
  const canNext = $derived.by(() => {
    if (stepIdx === 1) return scriptsRoot.trim().length > 0;
    return true; // step 0 (welcome) and step 2 (profile — optional) never block
  });

  async function next() {
    if (!canNext) return;
    // R4: "Next" is the natural click — it must persist a typed-but-unsaved Scripts root instead
    // of discarding it (the separate Save button used to be the only writer).
    if (stepIdx === 1 && scriptsRoot.trim() && !scriptsSaved) {
      saveErr = '';
      try {
        await saveScripts();
      } catch (e) {
        saveErr = String(e);
        return; // stay on the step — the path was NOT persisted
      }
    }
    // Re-probe whenever we leave the scripts step with a root: "I don't have these scripts" (or a
    // failed first probe) must not stick for the rest of the wizard once a real root is supplied —
    // this also self-heals the case where the user fixed the path outside the app.
    if (stepIdx === 1 && scriptsRoot.trim()) {
      scriptsPresent = await scriptsAvailable().catch(() => null);
    }
    if (stepIdx < TOTAL - 1) stepIdx += 1;
  }
  function back() {
    if (stepIdx > 0) stepIdx -= 1;
  }
  function finish(runCheck: boolean) {
    onFinish(runCheck);
  }
</script>

<ModalShell {open} onClose={() => finish(false)} size="md" closeOnBackdrop={false}>
  <div class="ob">
    <div class="ob-progress">{t('onboarding.step', { n: stepIdx + 1, total: TOTAL })}</div>

    {#if stepIdx === 0}
      <h3 class="dlg-h">{t('onboarding.welcomeTitle')}</h3>
      <p class="ob-body">{t('onboarding.welcomeBody')}</p>
      <p class="ob-hint">{t('onboarding.welcomeHint')}</p>
    {:else if stepIdx === 1}
      <h3 class="dlg-h">{t('onboarding.scriptsTitle')}</h3>
      <p class="ob-body">{t('onboarding.scriptsBody')}</p>
      <div class="dlg-fld">
        <span>{t('onboarding.scriptsLabel')}</span>
        <div class="ob-row">
          <FolderField bind:value={scriptsRoot} placeholder={t('onboarding.scriptsPlaceholder')} />
        </div>
        {#if saveErr}
          <span class="dlg-warn">{saveErr}</span>
        {:else if !scriptsRoot.trim()}
          <span class="dlg-hint">{t('onboarding.scriptsNeeded')}</span>
        {/if}
      </div>
      <button class="ob-link" onclick={skipScripts}>{t('onboarding.noScriptsBtn')}</button>
    {:else if stepIdx === 2}
      <h3 class="dlg-h">{t('onboarding.profileTitle')}</h3>
      <p class="ob-body">{t('onboarding.profileBody')}</p>
      <p class="ob-hint">
        {profileCount > 0
          ? t('onboarding.profileExisting', { n: profileCount })
          : t('onboarding.profileNoneYet')}
      </p>
      <div class="dlg-fld">
        <span>{t('profiles.dlgName')}</span>
        <div class="ob-row">
          <input
            class="sw-input ob-grow"
            bind:value={profName}
            placeholder={t('profiles.dlgNamePlaceholder')}
            spellcheck="false"
            autocomplete="off"
          />
          <button class="sw-btn sw-btn-primary" disabled={!nameValid || busy || scriptsPresent === false} onclick={createProfile}>
            {t('common.add')}
          </button>
        </div>
        {#if scriptsPresent === false}
          <span class="dlg-hint">{t('onboarding.noScriptsNote')}</span>
        {:else if profName && !nameValid}
          <span class="dlg-warn">{t('profiles.dlgNameError')}</span>
        {/if}
      </div>
      <button class="ob-link" onclick={onOpenProfiles}>{t('onboarding.profileOpenTab')}</button>
      <p class="ob-hint">{t('onboarding.profileSkipHint')}</p>
    {:else}
      <h3 class="dlg-h">{t('onboarding.doneTitle')}</h3>
      <p class="ob-body">{scriptsPresent === false ? t('onboarding.noScriptsNote') : t('onboarding.doneBody')}</p>
    {/if}

    <div class="ob-foot">
      <button class="sw-btn sw-btn-ghost" onclick={() => finish(false)}>{t('onboarding.skip')}</button>
      <div class="ob-foot-right">
        {#if stepIdx > 0}
          <button class="sw-btn sw-btn-ghost" onclick={back}>{t('onboarding.back')}</button>
        {/if}
        {#if stepIdx < TOTAL - 1}
          <button class="sw-btn sw-btn-primary" disabled={!canNext} onclick={next}>{t('onboarding.next')}</button>
        {:else if scriptsPresent === false}
          <button class="sw-btn sw-btn-primary" onclick={() => finish(false)}>{t('onboarding.doneJustFinish')}</button>
        {:else}
          <button class="sw-btn sw-btn-ghost" onclick={() => finish(false)}>{t('onboarding.doneJustFinish')}</button>
          <button class="sw-btn sw-btn-primary" onclick={() => finish(true)}>{t('onboarding.doneRunCheck')}</button>
        {/if}
      </div>
    </div>
  </div>
</ModalShell>

<style>
  .ob {
    display: flex;
    flex-direction: column;
  }
  .ob-progress {
    font-size: var(--sw-text-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--sw-text-muted);
    margin-bottom: var(--sw-space-2);
  }
  .ob-body {
    font-size: var(--sw-text-sm);
    color: var(--sw-text-secondary);
    line-height: 1.5;
    margin: 0 0 var(--sw-space-3);
  }
  .ob-hint {
    font-size: var(--sw-text-xs);
    color: var(--sw-text-muted);
    line-height: 1.5;
    margin: 0 0 var(--sw-space-2);
  }
  .ob-row {
    display: flex;
    align-items: center;
    gap: var(--sw-space-2);
  }
  .ob-grow {
    flex: 1;
    min-width: 0;
  }
  .ob-link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0;
    margin: 0 0 var(--sw-space-2);
    font-size: var(--sw-text-xs);
    color: var(--sw-accent-text);
    cursor: pointer;
    text-decoration: underline;
  }
  .ob-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sw-space-2);
    margin-top: var(--sw-space-6);
  }
  .ob-foot-right {
    display: flex;
    gap: var(--sw-space-2);
  }
</style>
