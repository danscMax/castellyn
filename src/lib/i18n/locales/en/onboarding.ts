// First-run onboarding wizard (OnboardingWizard.svelte): a short multi-step
// modal that walks a fresh user through the minimum setup (Scripts root + a
// profile) before they land on an empty Updates tab.
export default {
  // Progress + shell
  step: 'Step {n} of {total}',
  skip: 'Skip',
  back: 'Back',
  next: 'Next',
  finish: 'Finish',

  // Step 1 — welcome
  welcomeTitle: 'Welcome to Castellyn',
  welcomeBody:
    'Castellyn is the control center for your local Claude Code stack — updates, GitHub forks, profiles, MCP servers, providers and schedules, all in one place.',
  welcomeHint: 'A couple of quick steps and you are set up. You can skip and do this later in Settings.',

  // Step 2 — Scripts root
  scriptsTitle: 'Point at your Scripts folder',
  scriptsBody:
    'Castellyn drives your PowerShell maintenance scripts. Choose the folder that holds them (it contains the Castellyn subfolder).',
  scriptsLabel: 'Scripts root',
  scriptsPlaceholder: 'e.g. E:\\Scripts',
  scriptsNeeded: 'Pick a folder to continue.',

  // Step 3 — profile
  profileTitle: 'Set up a profile',
  profileBody:
    'Profiles are isolated Claude Code setups (separate logins, settings and shared folders). Create your first one, or open the Profiles tab to manage them.',
  profileExisting: '{n} profile(s) found.',
  profileNoneYet: 'No profiles yet.',
  profileOpenTab: 'Open Profiles tab',
  profileSkipHint: 'You can add profiles any time from the Profiles tab.',

  // Step 4 — finish
  doneTitle: 'All set',
  doneBody: 'Setup is complete. Run a first check to see what needs updating across the stack.',
  doneRunCheck: 'Finish and check for updates',
  doneJustFinish: 'Finish'
};
