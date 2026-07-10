import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// White text on a coloured fill is the one contrast failure a screenshot never shows and a regex
// contrast checker never catches — these colours compute to oklch() in the browser. So check the
// SOURCE instead: every rule that paints white text must paint it on a fill that clears WCAG AA.
//
// This also guards the undefined-token class of bug (a `var(--sw-bg)` that was never declared
// silently renders transparent, which once shipped).

const SRC = fileURLToPath(new URL('../', import.meta.url));
const APP_CSS = join(SRC, 'app.css');
const AA_TEXT = 4.5;

function relLum(r: number, g: number, b: number): number {
  const f = (v: number) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

function contrastWithWhite(rgb: [number, number, number]): number {
  return 1.05 / (relLum(...rgb) + 0.05);
}

/** #abc / #aabbcc / #aabbccdd → rgb + alpha; rgb()/rgba() → rgb + alpha. Anything else → null. */
function parseColor(raw: string): { rgb: [number, number, number]; a: number } | null {
  const s = raw.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3,8})$/.exec(s);
  if (hex) {
    let h = hex[1];
    if (h.length === 3 || h.length === 4) h = [...h].map((c) => c + c).join('');
    if (h.length !== 6 && h.length !== 8) return null;
    const n = parseInt(h.slice(0, 6), 16);
    const a = h.length === 8 ? parseInt(h.slice(6), 16) / 255 : 1;
    return { rgb: [(n >> 16) & 255, (n >> 8) & 255, n & 255], a };
  }
  const fn = /^rgba?\(([^)]+)\)$/.exec(s);
  if (fn) {
    const p = fn[1].split(/[,/\s]+/).filter(Boolean).map(Number);
    if (p.length < 3 || p.slice(0, 3).some(Number.isNaN)) return null;
    return { rgb: [p[0], p[1], p[2]], a: p[3] ?? 1 };
  }
  return null;
}

function isWhite(v: string): boolean {
  const c = parseColor(v);
  return !!c && c.a > 0.9 && c.rgb.every((x) => x > 250);
}

/** First declaration wins: `:root` precedes the `html.light` overrides in app.css. */
function tokenMap(css: string): Map<string, string> {
  const m = new Map<string, string>();
  for (const [, name, value] of css.matchAll(/(--sw-[a-z0-9-]+)\s*:\s*([^;}]+)[;}]/gi)) {
    if (!m.has(name)) m.set(name, value.trim());
  }
  return m;
}

/** Resolves nested var() up to a small depth; returns a marker for tokens that don't exist. */
function resolveVars(value: string, tokens: Map<string, string>): string {
  let out = value;
  for (let i = 0; i < 5 && out.includes('var('); i++) {
    out = out.replace(/var\(\s*(--[a-z0-9-]+)\s*(?:,[^()]*)?\)/gi, (_, name: string) =>
      tokens.get(name) ?? `UNDEFINED(${name})`,
    );
  }
  return out;
}

function svelteFiles(dir: string): string[] {
  const out: string[] = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) out.push(...svelteFiles(p));
    else if (e.name.endsWith('.svelte')) out.push(p);
  }
  return out;
}

/** CSS a browser would see: app.css verbatim, plus each component's <style> block. */
function sheets(): { file: string; css: string }[] {
  const list = [{ file: 'app.css', css: readFileSync(APP_CSS, 'utf8') }];
  for (const f of svelteFiles(SRC)) {
    const src = readFileSync(f, 'utf8');
    for (const [, css] of src.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
      list.push({ file: f.slice(SRC.length).replace(/\\/g, '/'), css });
    }
  }
  return list;
}

/** The innermost { … } around `index`; declarations never nest, so the nearest braces are ours. */
function enclosingBlock(css: string, index: number): { selector: string; body: string } | null {
  const open = css.lastIndexOf('{', index);
  const close = css.indexOf('}', index);
  if (open < 0 || close < 0) return null;
  const prev = Math.max(css.lastIndexOf('}', open), css.lastIndexOf('{', open - 1));
  return {
    selector: css.slice(prev + 1, open).replace(/\/\*[\s\S]*?\*\//g, '').trim().replace(/\s+/g, ' '),
    body: css.slice(open + 1, close),
  };
}

type Surface = { where: string; selector: string; bg: string; ratios: number[]; undefinedToken: string | null };

function whiteTextSurfaces(): Surface[] {
  const tokens = tokenMap(readFileSync(APP_CSS, 'utf8'));
  const found: Surface[] = [];

  for (const { file, css } of sheets()) {
    for (const m of css.matchAll(/(?:^|[;{])\s*color\s*:\s*([^;}]+)/gi)) {
      if (!isWhite(m[1])) continue;
      const block = enclosingBlock(css, m.index!);
      if (!block) continue;

      const bgm = /(?:^|[;{])\s*background(?:-color|-image)?\s*:\s*([^;}]+)/i.exec(block.body);
      if (!bgm) continue; // white text over whatever the parent paints — not decidable from source

      const resolved = resolveVars(bgm[1], tokens);
      const undef = /UNDEFINED\((--[a-z0-9-]+)\)/.exec(resolved);

      // Every colour stop of the fill (a gradient has several); an alpha fill is skipped, since
      // the backdrop it composites over is unknown here.
      const ratios: number[] = [];
      for (const [, stop] of resolved.matchAll(/(#[0-9a-f]{3,8}|rgba?\([^)]*\))/gi)) {
        const c = parseColor(stop);
        if (c && c.a > 0.9) ratios.push(contrastWithWhite(c.rgb));
      }
      found.push({
        where: file,
        selector: block.selector,
        bg: resolved.trim(),
        ratios,
        undefinedToken: undef ? undef[1] : null,
      });
    }
  }
  return found;
}

describe('white text never lands on a fill that fails WCAG AA', () => {
  const surfaces = whiteTextSurfaces();

  it('finds the known white-on-colour surfaces (guards the scanner itself)', () => {
    // If this drops to 0 the scanner broke, and every assertion below would vacuously pass.
    expect(surfaces.length).toBeGreaterThanOrEqual(5);
  });

  it('resolves every custom property it references', () => {
    const bad = surfaces.filter((s) => s.undefinedToken);
    expect(bad.map((s) => `${s.where} ${s.selector} → ${s.undefinedToken}`)).toEqual([]);
  });

  it('clears 4.5:1 on every opaque stop', () => {
    const bad = surfaces
      .filter((s) => s.ratios.length && Math.min(...s.ratios) < AA_TEXT)
      .map((s) => `${s.where} { ${s.selector} } ${s.bg} → ${s.ratios.map((r) => r.toFixed(2)).join('/')}`);
    expect(bad).toEqual([]);
  });
});

describe('contrast maths', () => {
  it('matches published WCAG ratios', () => {
    expect(contrastWithWhite([0, 0, 0])).toBeCloseTo(21, 1);
    expect(contrastWithWhite([255, 255, 255])).toBeCloseTo(1, 2);
    expect(contrastWithWhite([0x3b, 0x82, 0xf6])).toBeCloseTo(3.68, 1); // --sw-accent: a fill, not a text bed
    expect(contrastWithWhite([0x25, 0x63, 0xeb])).toBeCloseTo(5.17, 1); // --sw-accent-solid
  });

  it('expands short hex and reads alpha', () => {
    expect(parseColor('#fff')).toEqual({ rgb: [255, 255, 255], a: 1 });
    expect(parseColor('#00000080')?.a).toBeCloseTo(0.5, 1);
    expect(parseColor('rgba(37, 99, 235, 0.5)')).toEqual({ rgb: [37, 99, 235], a: 0.5 });
    expect(parseColor('linear-gradient')).toBeNull();
  });
});
