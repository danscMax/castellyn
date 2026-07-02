// First-run onboarding wizard (OnboardingWizard.svelte): a short multi-step
// modal that walks a fresh user through the minimum setup (Scripts root + a
// profile) before they land on an empty Updates tab.
export default {
  // Progress + shell
  step: 'Шаг {n} из {total}',
  skip: 'Пропустить',
  back: 'Назад',
  next: 'Далее',
  finish: 'Готово',

  // Step 1 — welcome
  welcomeTitle: 'Добро пожаловать в Castellyn',
  welcomeBody:
    'Castellyn — центр управления вашим локальным окружением Claude Code: обновления, форки GitHub, профили, MCP-серверы, провайдеры и расписания в одном месте.',
  welcomeHint: 'Пара быстрых шагов — и всё готово. Можно пропустить и настроить позже в Настройках.',

  // Step 2 — Scripts root
  scriptsTitle: 'Укажите папку Scripts',
  scriptsBody:
    'Castellyn запускает ваши PowerShell-скрипты обслуживания. Выберите папку, в которой они лежат (внутри неё есть подпапка Castellyn).',
  scriptsLabel: 'Папка Scripts',
  scriptsPlaceholder: 'например: E:\\Scripts',
  scriptsNeeded: 'Выберите папку, чтобы продолжить.',

  // Step 3 — profile
  profileTitle: 'Настройте профиль',
  profileBody:
    'Профили — это изолированные конфигурации Claude Code (отдельные логины, настройки и общие папки). Создайте первый или откройте вкладку «Профили».',
  profileExisting: 'Найдено профилей: {n}.',
  profileNoneYet: 'Профилей пока нет.',
  profileOpenTab: 'Открыть «Профили»',
  profileSkipHint: 'Профили можно добавить в любой момент на вкладке «Профили».',

  // Step 4 — finish
  doneTitle: 'Всё готово',
  doneBody: 'Настройка завершена. Запустите первую проверку, чтобы увидеть, что нужно обновить.',
  doneRunCheck: 'Готово и проверить обновления',
  doneJustFinish: 'Готово'
};
