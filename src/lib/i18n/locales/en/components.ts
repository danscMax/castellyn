// Display names for the maintenance components declared in manifest/maintenance-manifest.json.
// The manifest is read from disk at runtime and may carry components we ship no translation for,
// so every lookup falls back to the manifest's own `name` (see lib/componentLabel.ts).
export default {
  groups: {
    orchestrator: 'Orchestrator',
    claude: 'Claude',
    git: 'Git',
    cli: 'CLI tools',
    maintenance: 'Maintenance'
  },
  all: { name: 'Update everything' },
  plugins: { name: 'Claude plugins' },
  forks: { name: 'Forks (fork-sync)' },
  rtk: { name: 'RTK CLI' },
  speckit: { name: 'SpecKit commands' },
  opencode: { name: 'opencode CLI' },
  ccrrouter: { name: 'Claude Code Router' },
  freellmapi: { name: 'FreeLLMAPI proxy' },
  cargo: { name: 'Cargo binaries' },
  bomfix: { name: 'Config BOM fix' }
};
