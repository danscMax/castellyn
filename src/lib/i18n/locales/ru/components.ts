// Display names for the maintenance components declared in manifest/maintenance-manifest.json.
// The manifest is read from disk at runtime and may carry components we ship no translation for,
// so every lookup falls back to the manifest's own `name` (see lib/componentLabel.ts).
export default {
  all: { name: 'Обновить всё' },
  plugins: { name: 'Плагины Claude' },
  forks: { name: 'Форки (fork-sync)' },
  rtk: { name: 'RTK CLI' },
  speckit: { name: 'SpecKit команды' },
  opencode: { name: 'opencode CLI' },
  ccrrouter: { name: 'Claude Code Router' },
  freellmapi: { name: 'FreeLLMAPI прокси' },
  cargo: { name: 'Cargo-бинарники' },
  bomfix: { name: 'BOM-fix конфигов' }
};
