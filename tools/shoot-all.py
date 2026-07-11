#!/usr/bin/env python
"""DEV-only full-UI screenshot capture for design review.

Like shoot.py but sweeps EVERY tab in BOTH themes into plans/ui-review/<date>/ (gitignored),
so a design panel can critique the whole surface from real pixels. Needs `npm run dev` running.

Usage:  python tools/shoot-all.py <out-dir> [http://localhost:5173]
"""
import sys
from pathlib import Path
from playwright.sync_api import sync_playwright

OUT = Path(sys.argv[1])
BASE = sys.argv[2] if len(sys.argv) > 2 else "http://localhost:5173"

# Sidebar default order (navOrder.svelte.ts ORD_VER 5, grouped) -> .nav-item index.
# KEEP IN SYNC with NAV_GROUPS in navOrder.svelte.ts: inserting a tab shifts every later index.
NAV_INDEX = {
    "home": 0, "sessions": 1, "profiles": 2, "providers": 3, "mcp": 4,
    "envs": 5, "extensions": 6, "agents": 7, "updates": 8, "forks": 9,
    "backup": 10, "sync": 11, "schedule": 12, "analytics": 13, "settings": 14,
}

INIT_TMPL = """
localStorage.setItem('cmh-theme', '%THEME%');
localStorage.setItem('cmh-language', 'en');
localStorage.setItem('cmh-onboarded', '1');
localStorage.setItem('cmh-sidebar-order-ver', '5');
localStorage.setItem('cmh-sidebar-groups-closed', '{}');
const _gc = HTMLCanvasElement.prototype.getContext;
HTMLCanvasElement.prototype.getContext = function (type) {
  if (type === 'webgl' || type === 'webgl2' || type === 'experimental-webgl') return null;
  return _gc.apply(this, arguments);
};
"""


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        for theme in ("dark", "light"):
            ctx = browser.new_context(viewport={"width": 1360, "height": 900}, device_scale_factor=2)
            ctx.add_init_script(INIT_TMPL.replace("%THEME%", theme))
            for tab, idx in NAV_INDEX.items():
                page = ctx.new_page()
                page.goto(f"{BASE}/?shot", wait_until="networkidle")
                page.wait_for_selector(".nav-item", timeout=15000)
                page.add_style_tag(content=".toast-host{display:none!important}")
                page.wait_for_timeout(700)
                page.locator(".nav-item").nth(idx).click()
                page.wait_for_timeout(1300)
                if tab == "sessions":
                    try:
                        page.get_by_role("button", name="Launch").first.click()
                        page.wait_for_timeout(600)
                        page.get_by_role("button", name="Launch").first.click()
                        page.wait_for_timeout(3000)
                    except Exception as e:  # noqa: BLE001 - best-effort, keep sweeping
                        print(f"    (sessions launch skipped: {e})")
                dest = OUT / f"{theme}-{tab}.png"
                page.screenshot(path=str(dest))
                page.close()
                print(f"  {theme:5s} {tab:11s} -> {dest.name}")
            ctx.close()
        browser.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
