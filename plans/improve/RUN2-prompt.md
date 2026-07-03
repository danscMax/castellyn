# Прогон 2 · ФАЗА A — готовый промпт для свежей сессии /max:dev

Открой НОВУЮ сессию Claude Code в `E:\Scripts\Castellyn` и вставь блок ниже целиком.

> **Статус 2026-07-03:** живой смоук Run 1 вскрыл+починил 6 багов (`f96fb5d`). Перед Фазой A:
> ре-смоук 6 фиксов зелёный → `git switch main && git merge --ff-only auto-loop/improve-0703` → база `main`.
> **Это Фаза A** (redesign-safe подмножество). Полный порядок работ (Фаза A → herdr-редизайн Фаза B) —
> в `plans/improve/master-plan-run2-redesign.md`. Пункты 10/13/21d-чип НЕ здесь — они в редизайне.

---

/max:dev Прогон 2 плана улучшений Castellyn — Autonomous Plan-Run Mode (headless, без AskUserQuestion).

- **План:** `plans/improve/fix_plan-2026-07-03.json` + `plans/improve/master-plan-run2-redesign.md` (фазовое разбиение). Выполнить **только ФАЗУ A** — redesign-safe подмножество Прогона 2 (все `approved:true`, решения в `decision_resolved`): **#5, 11, 15, 17, 18, 19, 21c, 21e** + из **21d** только колонка utilization в **Профилях** (ProfilesTab). **НЕ делать** `10`, `13`, и чип-часть `21d` — они привязаны к верстке Сессий и вплавляются в herdr-редизайн (Фаза B). НЕ перезапускать Прогон 1 (Волны 1–3 кроме #5, 21a/21b) — уже на ветке.
- **#18 = ДЫРА 1** (root-cause, не костыль): durable sidecar `~/.claude/castellyn/sessions.json` (НЕ HubConfig) + миграция из localStorage при первом чтении (файл=истина, localStorage=зеркало) + добавить путь в Syncthing-whitelist (`sync_item_lines` ~lib.rs:1792) и в снимок бэкапа (`run_backup` ~lib.rs:1332) + read-on-focus. **Обязательно:** работает И в Castellyn, И standalone; НЕ трогает линкованные папки (skills/projects/agents/commands/hooks/plugins).
- **Git preflight (блокирующий):** база = ветка `auto-loop/improve-0703` (Run 1, зелёная, не влита; ИЛИ `main`, если Run 1 уже влит). Ответвить Run 2 → `auto-loop/improve-0703-run2`. НИКОГДА не стейджить/коммитить `.serena/project.yml` (чужой файл). Зелёный baseline сначала: `npm run check` 0/0, `npm test`, `cargo test`.
- **Oracle-Loop на каждый пункт:** контракт → изолированный Opus-имплементер → `verify`-оракул пункта + полные гейты проекта → независимый Opus-ревьюер по рубрике (APPROVE/REJECT с file:line). Oracle-freeze (тесты/оракул только для чтения). Полный гейт после каждой волны; красный → откат всей волны; зелёный → коммит по pathspec.
- **Порядок:** **18 первым** (durable sidecar — фундамент, на нём стоит и будущая персонализация редизайна), затем #5, 17, 15, 11, 19, 21c, 21e, 21d-Профили. Пункты, трогающие `SessionsTab.svelte` (11, 21c, части 18) — сериализовать (`⚠ same-file`), не параллелить.
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
