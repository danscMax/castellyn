#!/usr/bin/env python
"""DEV-only README screenshot capture.

Drives the vite dev server (started separately: `npm run dev`) against the mocked Tauri IPC
layer (see src/lib/shot/fixtures.ts + the `?shot` guard in src/routes/+layout.ts) and saves one
2720x1800 PNG per tab (DPR 2, 1360x900 logical) into docs/img/. Re-run for future releases.

Usage:  python tools/shoot.py [http://localhost:5173]
"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:5173"
OUT = Path(__file__).resolve().parent.parent / "docs" / "img"

# Sidebar default order (navOrder.svelte.ts ORD_VER 5, grouped) -> .nav-item index per tab.
# INIT below opens every sidebar group so the index always addresses a rendered item.
# KEEP IN SYNC with NAV_GROUPS in navOrder.svelte.ts: inserting a tab shifts every later index.
NAV_INDEX = {
    "home": 0, "sessions": 1, "profiles": 2, "providers": 3, "mcp": 4,
    "envs": 5, "extensions": 6, "agents": 7, "updates": 8, "forks": 9,
    "backup": 10, "sync": 11, "schedule": 12, "analytics": 13, "settings": 14,
}
# Same guard shoot-all.py carries: indices must be a contiguous 0..N-1 run, so a mis-edit fails
# loudly instead of silently screenshotting the WRONG tab under the right README filename.
assert list(NAV_INDEX.values()) == list(range(len(NAV_INDEX))), "NAV_INDEX out of sync with the sidebar"

# tab id -> output filename stem
SHOTS = {
    "profiles": "screenshot-profiles",
    "extensions": "screenshot-plugins-skills",
    "agents": "screenshot-subagents",
    "mcp": "screenshot-mcp",
    "envs": "screenshot-environments",
    "providers": "screenshot-providers",
    "sync": "screenshot-sync",
    "sessions": "screenshot-sessions",
    "forks": "screenshot-forks",
}
assert SHOTS.keys() <= NAV_INDEX.keys(), "SHOTS names a tab the sidebar map doesn't know"

# Seed localStorage so first-run UI (onboarding) is skipped and theme/order are deterministic.
INIT = """
localStorage.setItem('cmh-theme', 'dark');
localStorage.setItem('cmh-language', 'en');
localStorage.setItem('cmh-onboarded', '1');
localStorage.setItem('cmh-sidebar-order-ver', '5');
localStorage.setItem('cmh-sidebar-groups-closed', '{}');
// Force xterm onto its DOM renderer (disable WebGL): the WebGL canvas renderer does not reliably
// paint programmatically-written content into headless screenshots; the DOM renderer does.
const _gc = HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.getContext = function (type) {
  if (type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') return null;
  return _gc.apply(this, arguments);
};
"""


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        # WebGL is disabled in INIT (below) so xterm uses its DOM renderer, which paints into headless
        # screenshots; no head needed.
        browser = p.chromium.launch(headless=True)
        ctx = browser.new_context(viewport={"width": 1360, "height": 900}, device_scale_factor=2)
        ctx.add_init_script(INIT)

        # Fresh page per tab so run-lock / run-log state never bleeds across screenshots.
        for tab, stem in SHOTS.items():
            page = ctx.new_page()
            page.goto(f"{BASE}/?shot", wait_until="networkidle")
            page.wait_for_selector(".nav-item", timeout=15000)
            # Hide transient run-outcome toasts (injected into <head>, where !important reliably wins).
            page.add_style_tag(content=".toast-host{display:none!important}")
            page.wait_for_timeout(700)  # initial data load
            page.locator(".nav-item").nth(NAV_INDEX[tab]).click()
            page.wait_for_timeout(1300)  # tab data load + render
            if tab == "sessions":
                # Launch two panes so the grid shows real running terminals (flagship feature).
                # Guard the clicks (mirrors shoot-all.py): a missing Launch button skips only the
                # sessions shot instead of aborting the whole run.
                try:
                    launch = page.get_by_role("button", name="Launch").first
                    launch.click(); page.wait_for_timeout(700)
                    page.get_by_role("button", name="Launch").first.click()
                    page.wait_for_timeout(3200)  # xterm fit + per-line streamed output + paint
                except Exception as e:
                    print(f"  sessions launch skipped: {e}")
            dest = OUT / f"{stem}.png"
            page.screenshot(path=str(dest))
            page.close()
            print(f"  {tab:11s} -> {dest.name}")

        ctx.close()
        browser.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
