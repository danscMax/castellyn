export default {
  // Wave-4 config-drift / time keys
  configDrift: '配置文件',
  configDriftDesc: '共享配置（statusline、CLAUDE.md、RTK.md、hooks…）链接到跨机器同步的部署源。',
  driftedBadge: '{n} 个漂移',
  unlinkedBadge: '{n} 个未链接',
  configOk: '已同步',
  checkedAt: '检查于 {time}',
  driftCheckBtn: '重新检查',
  driftCheckTip: '重新扫描共享配置文件的漂移',
  syncNowBtn: '立即同步',
  syncNowTip: '将实时配置镜像到部署源（备份）',
  relinkBtn: '修复链接',
  relinkTip: '重新建立共享配置符号链接（需要管理员）',
  conflictsDesc: '检测到 Syncthing 冲突文件。',
  fstateOk: '正常',
  fstateLinked: '已链接',
  fstateMaster: '源',
  fstateDrifted: '已漂移',
  fstateUnlinked: '未链接',
  cleanConflictsBtn: '清理',
  cleanConflictsTip: '删除 *.sync-conflict-* 文件',
  conflictShow: '显示…',
  keepLocal: '保留本地',
  keepOther: '采用另一台的',
  stConfiguredButDown: '已配置但无法访问',
  memoTitle: '哪些内容不会同步',
  memoBody:
    '• 密钥与凭据 — 凭据管理器、settings*.json、.credentials.json（各机器本地保存）。\n• 插件与市场 — 需在每台电脑分别安装。\n• OpenCode/Codex 分发与「共享技能」— 需在每台机器重新执行。',
  // Header
  title: '跨设备同步',
  subtitle:
    '让 Claude Code 设置在你所有 PC 上保持一致：历史、会话 (/resume)、技能、代理、命令和快捷键会通过 Syncthing 自动复制（P2P，无云端）。下面是具体要同步的内容。',
  refreshTitle: '重新读取 Syncthing 状态和同步设置',
  refreshing: '处理中…',
  refresh: '刷新',

  // Byte units (KB/MB/… ladder), comma-joined so t() returns a string.
  byteUnits: 'B,KB,MB,GB,TB',

  // Syncthing state labels
  stateIdle: '正常 (idle)',
  stateSyncing: '同步中…',
  stateScanning: '扫描中…',
  stateError: '错误',

  // Syncthing status card
  syncthing: 'Syncthing',
  daemonTitle: 'Syncthing 守护进程可通过本地 REST 访问',
  connected: '已连接',
  openWebUi: 'Web UI',
  openWebUiTip: '打开 Syncthing 网页界面（localhost:8384）',
  notFoundTitle: '未找到或未运行 Syncthing——此设备上同步未激活',
  notFound: '未找到',
  folder: '文件夹',
  folderIdTitle: '文件夹 ID：{id}',
  state: '状态',
  completion: '就绪度',
  completionTitle: '已同步数据的比例',
  connectedDevices: '已连接的其他设备',
  connectedDevicesTitle:
    'Syncthing 当前看到在线的其他设备数量。本设备不计入，因此另一台设备 = 1。',
  folderNotShared: '此设备上 ~/.claude 文件夹未添加到 Syncthing。',
  noSyncthingYet: '下面的设置会保存到 .stignore，并在 Syncthing 出现后立即生效。',

  // Drift warning
  needsApplyBadge: '需要应用',
  driftWarning: '已部署的 .stignore 与下面的设置不一致——请点击「应用」。',

  // Item toggles
  whatToSync: '同步哪些内容',
  itemTitle: '将「{path}」一行加入 .stignore 白名单',
  itemToggleTip: '开启/关闭此项目在设备间的同步；本地文件不受影响',
  applyTitle: '将选择保存到 sync-config.json，重新生成 .stignore 并请求 Syncthing 重新扫描',
  apply: '应用',
  unsavedChanges: '有未保存的更改',
  allApplied: '已全部应用',
  footnote:
    '禁用某项只会停止在设备间同步它——本地文件不会被删除。密钥、settings.json 和插件缓存永不同步。',

  // Items
  itemHistoryLabel: '命令历史',
  itemHistoryDesc: '已输入命令的列表',
  itemProjectsLabel: '会话与记忆',
  itemProjectsDesc: '会话 (/resume) 和项目原生记忆',
  itemSkillsLabel: '技能',
  itemSkillsDesc: '个人技能',
  itemAgentsLabel: '代理',
  itemAgentsDesc: '自定义子代理',
  itemCommandsLabel: '命令',
  itemCommandsDesc: '斜杠命令',
  itemKeybindingsLabel: '快捷键',
  itemKeybindingsDesc: '按键布局',
  itemCastellynLabel: '会话设置',
  itemCastellynDesc: '预设、收藏、布局、文件夹、参数',

  // Drift diff (Phase 3.2)
  showDiff: '显示差异',
  hideDiff: '隐藏差异',
  diffTitle: '与共享副本的差异',

  // Empty state
  emptyTitle: '无数据',
  emptyHint: '点击「刷新」以收集同步状态。'
};
