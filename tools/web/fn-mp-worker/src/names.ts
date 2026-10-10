const SUBSTR = [
  'fuck', 'shit', 'cunt', 'bitch', 'whore', 'slut', 'pussy', 'penis', 'vagina', 'porn', 'rape',
  'niger', 'niga', 'nigr', 'faget', 'fagot', 'retard', 'trany', 'kike', 'chink', 'wetback',
  'beaner', 'raghead', 'towelhead', 'hitler', 'nazi', 'pedophil', 'molest', 'jizz', 'twat', 'wank', 'asshole', 'killyourself',
];
const WORD = ['fuk', 'fck', 'fag', 'fags', 'dick', 'cock', 'cum', 'anal', 'anus', 'spic', 'coon', 'gok', 'dyke', 'paki', 'kkk', 'heil', 'isis', 'kys', 'rapist', 'jihad', 'bastard', 'pedo', 'pedos'];
const LEET: Record<string, string> = { '0': 'o', '1': 'i', '3': 'e', '4': 'a', '5': 's', '7': 't', '8': 'b', '9': 'g', '$': 's', '@': 'a', '!': 'i', '|': 'i' };
const CHARSET = /^[A-Za-z0-9 _.\-]+$/;

function fold(s: string): string {
  let out = '';
  for (const ch of s.toLowerCase()) { const c = LEET[ch] ?? ch; out += c >= 'a' && c <= 'z' ? c : ' '; }
  return out.replace(/([a-z])\1+/g, '$1');
}
const dedupe = (w: string) => w.replace(/([a-z])\1+/g, '$1');

export function badWord(word: string): boolean {
  return fold(word).split(' ').filter(Boolean).some((f) => SUBSTR.some((w) => f.includes(dedupe(w))) || WORD.some((w) => f === dedupe(w)));
}

export function cleanName(raw: unknown): string | null {
  if (typeof raw !== 'string') return null;
  const name = raw.normalize('NFKC').trim().replace(/\s+/g, ' ');
  if (name.length < 3 || name.length > 16 || !CHARSET.test(name)) return null;
  if (!/[A-Za-z0-9]/.test(name)) return null;
  const words = fold(name).split(' ').filter(Boolean);
  const joined = words.join('');
  for (const w of SUBSTR) if (joined.includes(dedupe(w))) return null;
  for (const w of WORD) { const d = dedupe(w); if (words.includes(d) || joined === d) return null; }
  return name;
}
