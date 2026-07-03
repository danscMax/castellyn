# Run 2 — готовый промпт для свежей сессии /max:dev

Открой НОВУЮ сессию Claude Code в `E:\Scripts\Castellyn` и вставь блок ниже целиком.

> **Перед этим (на владельце):** реши, что делаешь с Run 1 —
> A) живой смоук Run 1 → FF-merge в main → Run 2 от main (рекомендуется);
> B) merge сейчас без смоука; C) Run 2 поверх ветки, смоук в конце.
> Если выбрал A и смоук зелёный — сначала: `git switch main && git merge --ff-only auto-loop/improve-0703`,
> и в промпте ниже поменяй базу на `main`.

---

/max:dev Прогон 2 плана улучшений Castellyn — Autonomous Plan-Run Mode (headless, без AskUserQuestion).

- **План:** `plans/improve/fix_plan-2026-07-03.json`. Выполнить ТОЛЬКО 11 пунктов Прогона 2 (все `approved:true`, решения зафиксированы в `decision_resolved`): **#5, 10, 11, 13, 15, 17, 18, 19, 21c, 21d, 21e**. НЕ перезапускать пункты Прогона 1 (Волны 1–3 кроме #5, плюс 21a/21b) — они уже закоммичены на ветке.
- **#18 = ДЫРА 1** (root-cause, не костыль): durable sidecar `~/.claude/castellyn/sessions.json` (НЕ HubConfig) + миграция из localStorage при первом чтении (файл=истина, localStorage=зеркало) + добавить путь в Syncthing-whitelist (`sync_item_lines` ~lib.rs:1792) и в снимок бэкапа (`run_backup` ~lib.rs:1332) + read-on-focus. **Обязательно:** работает И в Castellyn, И standalone; НЕ трогает линкованные папки (skills/projects/agents/commands/hooks/plugins).
- **Git preflight (блокирующий):** база = ветка `auto-loop/improve-0703` (Run 1, зелёная, не влита; ИЛИ `main`, если Run 1 уже влит). Ответвить Run 2 → `auto-loop/improve-0703-run2`. НИКОГДА не стейджить/коммитить `.serena/project.yml` (чужой файл). Зелёный baseline сначала: `npm run check` 0/0, `npm test`, `cargo test`.
- **Oracle-Loop на каждый пункт:** контракт → изолированный Opus-имплементер → `verify`-оракул пункта + полные гейты проекта → независимый Opus-ревьюер по рубрике (APPROVE/REJECT с file:line). Oracle-freeze (тесты/оракул только для чтения). Полный гейт после каждой волны; красный → откат всей волны; зелёный → коммит по pathspec.
- **Порядок:** #5 первым (остаток Волны 3), затем Волна 4 (10/11/13/15/17/18/19), затем 21c/21d/21e. Пункты в одном файле `SessionsTab.svelte` (11/12-нет/16-нет/21c/21d + 3/18 части) — сериализовать (`⚠ same-file`), не параллелить.
- **Бюджет:** burn tokens (глубокая адверсариальная верификация).
- **Только живой смоук (пометить мне, НЕ фейкать):** 21c/21e нужен реальный `limited`-сеанс; 18 — реальный экспорт→импорт + факт файла под `~/.claude/castellyn/`; UI-пункты — живой запуск app (Tauri), не URL в браузере.
- **В конце:** блок `AUTOFIX COMPLETE` + итог простым языком: что изменилось, что мне тестировать живьём vs. что можно сразу мерджить.

---

## После Run 2 (владелец)
1. Живой смоук Run 2 (Сессии, лимиты, персонализация #18 экспорт/импорт).
2. `git switch main && git merge --ff-only auto-loop/improve-0703` (Run 1, если ещё не влит), затем merge Run 2.
3. `git push origin main` (сейчас локальный main на 1 коммит впереди origin).
4. Удалить стале-ветки: `feat/agent-status feat/plugin-sync feat/session-restore feat/status-notifications fix/status-review`.
5. `.\build_all.ps1` → релизный `castellyn.exe`.
