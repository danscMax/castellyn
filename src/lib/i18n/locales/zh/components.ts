// Display names for the maintenance components declared in manifest/maintenance-manifest.json.
// The manifest is read from disk at runtime and may carry components we ship no translation for,
// so every lookup falls back to the manifest's own `name` (see lib/componentLabel.ts).
export default {
  all: { name: '全部更新' },
  plugins: { name: 'Claude 插件' },
  forks: { name: '复刻仓库 (fork-sync)' },
  rtk: { name: 'RTK CLI' },
  speckit: { name: 'SpecKit 命令' },
  opencode: { name: 'opencode CLI' },
  ccrrouter: { name: 'Claude Code Router' },
  freellmapi: { name: 'FreeLLMAPI 代理' },
  cargo: { name: 'Cargo 二进制文件' },
  bomfix: { name: '配置 BOM 修复' }
};
