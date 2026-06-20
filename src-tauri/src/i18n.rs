//! Backend-side localization for user-facing strings the Rust layer produces:
//! command errors (shown as toasts), the run-log stream (console panel), and the
//! tray menu (a native surface the JS i18n tables can't reach).
//!
//! The frontend owns the locale (localStorage `cmh-language`); it mirrors the choice
//! into HubConfig.language via the `set_language` command, and lib.rs keeps the current
//! `Lang` in a process global that `tr`/`trv` read.
//!
//! Parity is STRUCTURAL: every entry is `[ru, en, zh]`, so a key cannot exist for one
//! language and not another (the array type forbids it). New strings: add one row.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
    Zh,
}

impl Lang {
    /// Map a frontend locale code to a Lang (unknown → Ru, the project's source language).
    pub fn parse(s: &str) -> Lang {
        match s {
            "en" => Lang::En,
            "zh" => Lang::Zh,
            _ => Lang::Ru,
        }
    }
    fn idx(self) -> usize {
        match self {
            Lang::Ru => 0,
            Lang::En => 1,
            Lang::Zh => 2,
        }
    }
}

// [ru, en, zh]. ru = the original hardcoded literal. Keep namespaces: tray.*, err.*, log.*, det.*.
#[rustfmt::skip]
const TABLE: &[(&str, [&str; 3])] = &[
    // ── tray ────────────────────────────────────────────────────────────────
    ("tray.show",      ["Показать окно", "Show window", "显示窗口"]),
    ("tray.check_all", ["Проверить всё", "Check all", "全部检查"]),
    ("tray.quit",      ["Выход", "Quit", "退出"]),
    ("tray.tooltip_sessions", ["Castellyn — активных сессий: {n}", "Castellyn — active sessions: {n}", "Castellyn — 活动会话: {n}"]),

    // ── config write (touched by language-preserving write_config) ───────────
    ("err.no_appdata",    ["APPDATA не найден", "APPDATA not found", "未找到 APPDATA"]),
    ("err.write_config",  ["запись config: {e}", "writing config: {e}", "写入 config: {e}"]),
];

/// Translate a key to `lang`. Unknown key → the key itself (visible-but-stable, like the JS t()).
pub fn tr(key: &str, lang: Lang) -> &'static str {
    for (k, vals) in TABLE {
        if *k == key {
            let v = vals[lang.idx()];
            return if v.is_empty() { vals[Lang::En.idx()] } else { v };
        }
    }
    // Leak nothing: return a 'static fallback. The key isn't 'static here, so callers that
    // need the key echoed use trv (owned String). For tr, an unknown key is a bug — surface it.
    "?"
}

/// Translate with `{name}` interpolation, mirroring the frontend interpolate(). Returns an owned
/// String. Unknown key → echoes the key so the miss is visible in the UI/log.
pub fn trv(key: &str, lang: Lang, vars: &[(&str, &str)]) -> String {
    let mut found = None;
    for (k, vals) in TABLE {
        if *k == key {
            let v = vals[lang.idx()];
            found = Some(if v.is_empty() { vals[Lang::En.idx()] } else { v });
            break;
        }
    }
    let mut s = match found {
        Some(t) => t.to_string(),
        None => key.to_string(),
    };
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_language() {
        assert_eq!(tr("tray.quit", Lang::Ru), "Выход");
        assert_eq!(tr("tray.quit", Lang::En), "Quit");
        assert_eq!(tr("tray.quit", Lang::Zh), "退出");
    }

    #[test]
    fn trv_interpolates() {
        assert_eq!(
            trv("tray.tooltip_sessions", Lang::En, &[("n", "3")]),
            "Castellyn — active sessions: 3"
        );
    }

    #[test]
    fn unknown_key_echoes_in_trv() {
        assert_eq!(trv("err.nope", Lang::En, &[]), "err.nope");
    }

    #[test]
    fn parse_maps_codes() {
        assert!(matches!(Lang::parse("en"), Lang::En));
        assert!(matches!(Lang::parse("zh"), Lang::Zh));
        assert!(matches!(Lang::parse("ru"), Lang::Ru));
        assert!(matches!(Lang::parse("xx"), Lang::Ru)); // unknown → source language
    }
}
