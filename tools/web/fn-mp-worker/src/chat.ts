import { badWord } from './names';

export const CHAT_MAX = 120;
const TLDS = new Set(['com', 'net', 'org', 'gg', 'io', 'xyz', 'ru', 'lol', 'tv', 'me', 'co', 'app', 'dev', 'link', 'ly', 'uk', 'de', 'info', 'biz', 'site', 'online', 'shop', 'store', 'cc']);

export function hasUrl(text: string): boolean {
  const l = text.toLowerCase();
  if (l.includes('://') || l.includes('www.')) return true;
  return l.split(/\s+/).some((raw) => {
    const w = raw.replace(/^[^a-z0-9]+|[^a-z0-9]+$/g, '');
    const parts = w.split('.');
    if (parts.length < 2 || !parts[0] || parts.slice(1).some((p) => !p)) return false;
    return TLDS.has(parts[parts.length - 1].split('/')[0]);
  });
}

export type ChatResult = { ok: true; text: string } | { ok: false; code: 'empty' | 'long' | 'url' | 'filter' };

export function cleanChat(raw: unknown): ChatResult {
  if (typeof raw !== 'string') return { ok: false, code: 'empty' };
  const text = raw.replace(/[^\x20-\x7e]|[<>]/g, '').trim().replace(/\s+/g, ' ');
  if (!text) return { ok: false, code: 'empty' };
  if (text.length > CHAT_MAX) return { ok: false, code: 'long' };
  if (hasUrl(text)) return { ok: false, code: 'url' };
  if (text.split(' ').some((w) => badWord(w))) return { ok: false, code: 'filter' };
  return { ok: true, text };
}
