export default {
  title: 'Плагины и скиллы',
  subtitle: 'Установленные плагины и личные скиллы Claude Code',
  refreshTip: 'Перечитать списки плагинов и скиллов',
  refreshing: 'Идёт…',
  refreshBtn: 'Обновить список',

  // Plugins section
  pluginsHeading: 'Плагины ({count})',
  withUpdateBadge: '{count} с обновлением',
  withUpdateBadgeTip: 'Плагинов с доступным обновлением',
  updateAvailableBadge: 'обновление есть',
  updateAvailableBadgeTip: 'Доступна версия {version}',
  managedBadge: 'managed',
  managedBadgeTip: 'Навязан managed-settings во всех профилях',
  enabledBadge: 'включён',
  disabledBadge: 'выключен',
  enabledTip: 'Плагин включён',
  disabledTip: 'Плагин выключен',

  // Plugin contents (details)
  contentsLabel: 'Состав:',
  contentsToggleTip: 'Развернуть/свернуть список скиллов, команд и агентов плагина',
  skillsBadge: '{count} скиллов',
  skillsBadgeTip: 'Скиллы внутри плагина',
  commandsBadge: '{count} команд',
  commandsBadgeTip: 'Слэш-команды плагина',
  agentsBadge: '{count} агентов',
  agentsBadgeTip: 'Сабагенты плагина',
  catSkills: 'Скиллы',
  catCommands: 'Команды',
  catAgents: 'Агенты',

  // Plugin actions
  updateBtn: 'Обновить',
  updateBtnTip: 'Доступна версия {version} — обновить (общий кэш, для всех профилей)',
  upToDate: 'актуально',
  upToDateTip: 'Обновлений не обнаружено',
  disableBtn: 'Выключить',
  disableBtnTip:
    'Выключить во всех профилях. managed-плагин может быть снова включён managed-settings при следующей сессии',
  enableBtn: 'Включить',
  enableBtnTip: 'Включить во всех профилях',
  noPlugins: 'Плагины не найдены (или claude CLI недоступен).',

  // Skills section
  skillsHeading: 'Скиллы ({count})',
  openSkillsBtn: 'Открыть папку скиллов',
  openSkillsTip: 'Открыть ~/.claude/skills в Проводнике',
  skillsNote:
    'Скиллы — это файлы в ~/.claude/skills (управляются файлами и плагинами; индивидуального вкл/выкл нет).',
  noSkills: 'Скиллы не найдены в ~/.claude/skills.'
};
