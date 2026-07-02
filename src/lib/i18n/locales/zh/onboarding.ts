// First-run onboarding wizard (OnboardingWizard.svelte): a short multi-step
// modal that walks a fresh user through the minimum setup (Scripts root + a
// profile) before they land on an empty Updates tab.
export default {
  // Progress + shell
  step: '第 {n} 步，共 {total} 步',
  skip: '跳过',
  back: '上一步',
  next: '下一步',
  finish: '完成',

  // Step 1 — welcome
  welcomeTitle: '欢迎使用 Castellyn',
  welcomeBody:
    'Castellyn 是你本地 Claude Code 环境的控制中心——更新、GitHub 复刻、配置档案、MCP 服务器、供应商和计划任务，尽在一处。',
  welcomeHint: '几个简单步骤即可完成设置。你也可以跳过，稍后在“设置”中完成。',

  // Step 2 — Scripts root
  scriptsTitle: '指定 Scripts 文件夹',
  scriptsBody:
    'Castellyn 会运行你的 PowerShell 维护脚本。请选择存放这些脚本的文件夹（其中包含 Castellyn 子文件夹）。',
  scriptsLabel: 'Scripts 根目录',
  scriptsPlaceholder: '例如：E:\\Scripts',
  scriptsNeeded: '请选择一个文件夹以继续。',

  // Step 3 — profile
  profileTitle: '设置配置档案',
  profileBody:
    '配置档案是相互隔离的 Claude Code 环境（独立的登录、设置和共享文件夹）。创建第一个档案，或打开“配置档案”标签页进行管理。',
  profileExisting: '已找到 {n} 个配置档案。',
  profileNoneYet: '暂无配置档案。',
  profileOpenTab: '打开“配置档案”',
  profileSkipHint: '你可以随时在“配置档案”标签页中添加档案。',

  // Step 4 — finish
  doneTitle: '全部就绪',
  doneBody: '设置已完成。运行一次检查，看看整个环境有哪些需要更新。',
  doneRunCheck: '完成并检查更新',
  doneJustFinish: '完成'
};
