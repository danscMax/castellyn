# Волна 3 — Надёжность

Ветка `improve-0703/wave-3`. Задачи 3.1/3.5 обе правят хук-прошивку в lib.rs — сериализуй их.

## 3.1 Гонка записи settings.json + частичная прошивка ⚠ same-file (с 3.5)
**Файл:** `src-tauri/src/lib.rs`, `agent_status_hook_set` :7671-7700 (цикл + write :7694-7696),
`plugin_sync_set` ~:7548-7567 (аналог), `plugin_sync_profiles` :7402.
Питон-писатель: `src-tauri/assets/plugin_sync.py` :72-75 (`os.replace(tmp, fp)`).
**Сейчас:** Castellyn (`write_json_atomic`) и `plugin_sync.py` (на каждом SessionStart живых
CC-сессий) пишут один settings.json last-writer-wins → одновременная запись теряет чужие
изменения. Плюс `?` внутри цикла по профилям (:7696): ошибка на профиле №2 из 3 обрывает
команду — состояние смешанное wired/unwired без отката/отчёта.
**Сделать:**
(а) при записи ловить sharing violation (Windows os error 32) → до 3 ретраев с backoff
50/150/400 мс; если не удалось — не `?`, а собрать в per-profile ошибку.
(б) заменить `?`-abort на continue-on-error: пройти все профили, накопить `Vec<(profile,
Result)>`, вернуть агрегат (сколько прошито / какие профили упали) — фронт покажет.
**needs_confirmation:** полноценный файловый лок (advisory .lock) — избыточно, бери ретрай.
**Тест:** юнит на continue-on-error (один профиль-путь неписуем → остальные прошиты, отчёт
содержит упавший). **Verify:** cargo test + вручную toggle при живой claude-сессии.

## 3.2 Restore-набор сессий не затирать до решения пользователя
**Файл:** `src/lib/components/SessionsTab.svelte` :340-358 (persist `$effect` :349),
restore-бар (:1316-1322), `savedLive`/`restorable` (:342, onMount :134-149).
**Сейчас:** persist-эффект пишет `LIVE_KEY` текущим (на холодном старте — пустым) списком сразу
после маунта (:349-354), затирая прошлый набор. Ctrl+R / крэш WebView2 до клика «восстановить»
→ `savedLive` перечитывает `[]` → набор (включая `--resume` claudeSid) потерян навсегда.
**Сделать:** пока `restorable` непуст И restore-бар не dismissed/не принят — НЕ перезаписывать
LIVE_KEY (или писать в отдельный ключ `cmh-sessions-live-pending`, а основной оставлять до
явного действия). После «восстановить» или «скрыть» — снять защиту и вести persist как обычно.
Не залипни на вечном старом наборе: dismiss обязан очищать.
**Verify:** ?shot/ручной: рестарт с сессиями → Ctrl+R до клика → бар всё ещё предлагает restore.

## 3.3 Single-instance guard
**Файлы:** `src-tauri/Cargo.toml` (dep `tauri-plugin-single-instance = "2"`),
`src-tauri/src/lib.rs` (регистрация плагина в билдере, рядом с прочими `.plugin(...)`),
возможно capability, если требует v2.
**Сейчас:** grep `single.instance` по src-tauri пуст — две копии Castellyn возможны: два
poll-треда agent_status (двойные звуки/тосты), гонки config.json, два трей-икона.
**Сделать:** подключить `tauri-plugin-single-instance`; в колбэке — сфокусировать/показать
существующее окно (учти start-hidden/close-to-tray: окно может быть скрыто в трее — показать +
`set_focus`). Проверь порядок init относительно autostart/tray.
**Проверь докой:** context7 / офиц. дока Tauri v2 single-instance — актуальный API колбэка.
**Verify:** запустить `castellyn.exe` дважды → одно окно поднимается на передний план, второй
процесс завершается. cargo test / build.

## 3.4 Эвристика blocked самосбрасывается до ответа
**Файл:** `src-tauri/src/agent_status.rs`, `compute` :188-197 (ветка `Some("blocked")`),
константа `BLOCKED_RESUME_MS` (=1500).
**Сейчас:** любой PTY-вывод спустя >1.5 с после Notification-хука (перерисовка промпта при
ресайзе окна, спиннер, countdown под промптом) переводит blocked→working, гася бейдж/звук
«ждёт решения», хотя промпт не отвечен. Эвристика предполагает ноль байтов после отрисовки.
**Сделать:** снимать `blocked` только по СЛЕДУЮЩЕМУ hook-событию (UserPromptSubmit/Stop —
достоверный сигнал ответа), а не по любому выводу. Чтобы не залипнуть, если пользователь
ответил в терминале, а хук почему-то не пришёл — оставить fallback по значительному burst'у
(например >N байт за короткое окно ИЛИ тишина сменилась устойчивым потоком), НЕ по единичному
чанку. Порог подобрать консервативно, отметить `ponytail:`-комментарием с потолком.
**needs_confirmation:** hook-only (чисто, но зависит от прихода хука) vs hook-first+byte-fallback
— бери второе (надёжнее). **Тест:** юнит compute: blocked + одиночный мелкий вывод через 2 с →
остаётся blocked; blocked + UserPromptSubmit-хук → working. **Verify:** cargo test.

## 3.5 Гигиена жизненного цикла хуков ⚠ same-file (с 3.1)
**Файл:** `src-tauri/src/lib.rs` (`plugin_sync_profiles` :7402, `agent_status_hook_set` :7671,
`ensure_status_hook_script`/`ensure_plugin_sync_script`, `hook_cmd_unwire`),
хук-команда `STATUS_HOOK_CMD` ~:7615 (`py -X utf8 …`).
**Три под-правки:**
(3.5a) Орфаны профилей: `plugin_sync_profiles` перечисляет только `.claude` + текущие имена из
profiles.json. Переименованный/удалённый профиль сохраняет прошитые хуки навсегда, невидимые
статусу. Сделать unwire-путь, который дополнительно сканирует существующие `~/.claude-*` дирки
и снимает ТОЛЬКО записи с нашим маркером (`STATUS_HOOK_MARKER`/plugin-sync marker) внутри их
settings.json. ⚠ ОСТОРОЖНО: НЕ трогать `.claude-mem` и подобные — фильтр строго по нашему
маркеру в содержимом, не по имени дирки (урок из [[herdr-port]]).
(3.5b) Удаление скриптов при выключении: выключение тумблеров сейчас только unwire'ит
settings.json; `~/.claude/hooks/castellyn_status.py` / `plugin_sync.py` остаются. Добавить
removal-пару: когда фича выключена И ни один профиль больше не прошит — удалить скрипт-файл.
(3.5c) py-health: `py -X utf8 …` может отсутствовать в hook-среде CC; хук fail-open →
статусы молча деградируют до activity-only, тумблер показывает «включено». Добавить пробу
(`py --version` при включении) и индикатор в ⚙ Сессий («Python-лаунчер не найден» если проба
провалилась). i18n ru/en/zh.
**needs_confirmation:** (3.5a) скан `.claude-*` — риск. Если решишь рискованным — сделай хотя бы
детект+предупреждение орфанов без авто-удаления. Отметь решение.
**Verify:** переименовать тест-профиль → unwire чистит хвост (или предупреждает); выключить →
скрипт удалён; сломать py в PATH → индикатор виден. cargo test.

## 3.6 mtime-гейт чтения hook-файлов + AtomicU64 last_output — PERF
**Файл:** `src-tauri/src/agent_status.rs`, poll-цикл :266-312 (чтение файлов :280-288),
`on_output` :87-95, struct `Track` :74.
**Сейчас:** каждые 500 мс читается+парсится hook-JSON КАЖДОГО claude-pane без mtime/size-гейта
(12 панелей ≈ 24 read+parse/с постоянно). `on_output` берёт глобальный `TRACKS`-мьютекс на
каждый 32-KiB чанк каждого reader-треда.
**Сделать:** (а) хранить в Track последний виденный mtime hook-файла; в poll читать
`std::fs::metadata(...).modified()` и парсить, ТОЛЬКО если mtime изменился (metadata дешевле
read+parse). (б) `last_output` перевести на `AtomicU64` (per-track или отдельная структура),
чтобы `on_output` обновлял его relaxed-store БЕЗ взятия TRACKS-лока. Сверься, что compute
по-прежнему видит согласованное значение.
**Тест:** юнит: неизменный mtime → парс не вызывается (через счётчик/флаг). **Verify:** cargo
test + живой смоук статусов (точки обновляются как раньше).

## Гейт волны
Все гейты + build_all.ps1 → ff-merge в main.
