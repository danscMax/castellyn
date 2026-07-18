import { describe, it, expect } from 'vitest';
import { guardedSend } from './guardedSend';

const BEGIN = '\x1b[200~';
const END = '\x1b[201~';

describe('guardedSend', () => {
  it('sent: idle throughout — bracketed paste then a separate Enter', async () => {
    const writes: string[] = [];
    const r = await guardedSend((d) => (writes.push(d), Promise.resolve()), 'hello', () => false, { delayMs: 0 });
    expect(r).toBe('sent');
    expect(writes).toEqual([`${BEGIN}hello${END}`, '\r']);
  });

  it('not-ready: busy at the start — nothing is written', async () => {
    const writes: string[] = [];
    const r = await guardedSend((d) => (writes.push(d), Promise.resolve()), 'hi', () => true, { delayMs: 0 });
    expect(r).toBe('not-ready');
    expect(writes).toEqual([]);
  });

  it('partial: busy appears between paste and Enter — text pasted, no Enter', async () => {
    const writes: string[] = [];
    let busy = false;
    const r = await guardedSend(
      (d) => {
        writes.push(d);
        busy = true; // agent becomes busy right after the paste lands
        return Promise.resolve();
      },
      'hi',
      () => busy,
      { delayMs: 0 }
    );
    expect(r).toBe('partial');
    expect(writes).toEqual([`${BEGIN}hi${END}`]); // paste only, no '\r'
  });

  it('enter:false — paste only (a reviewed snippet), busy-gate still applies', async () => {
    const writes: string[] = [];
    const r = await guardedSend((d) => (writes.push(d), Promise.resolve()), 'draft', () => false, {
      enter: false
    });
    expect(r).toBe('sent');
    expect(writes).toEqual([`${BEGIN}draft${END}`]); // no '\r'
    expect(await guardedSend((d) => (writes.push(d), Promise.resolve()), 'x', () => true, { enter: false })).toBe(
      'not-ready'
    );
  });

  it('sanitizes a nested paste-end so text cannot escape bracketed-paste mode', async () => {
    const writes: string[] = [];
    const r = await guardedSend((d) => (writes.push(d), Promise.resolve()), `a${END}evil`, () => false, { delayMs: 0 });
    expect(r).toBe('sent');
    expect(writes[0]).toBe(`${BEGIN}aevil${END}`);
    expect(writes[0].indexOf(END)).toBe(writes[0].length - END.length); // exactly one END, at the tail
  });
});
