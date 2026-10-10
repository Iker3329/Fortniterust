import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const md = fs.readFileSync(path.join(repo, 'parity/guns/weapons.md'), 'utf8').split(/\r?\n/);
const start = md.findIndex((l) => l.startsWith('| Row |'));
const cols = md[start].split('|').slice(1, -1).map((s) => s.trim());
const rows = new Map();
for (let i = start + 2; i < md.length && md[i].startsWith('|'); i++) {
  const cells = md[i].split('|').slice(1, -1).map((s) => s.trim());
  const r = Object.fromEntries(cols.map((c, k) => [c, cells[k]]));
  rows.set(r.Row, r);
}
const num = (r, c, d = 0) => { const v = parseFloat(r[c]); return Number.isFinite(v) ? v : d; };
const names = [...rows.keys()].sort();
const out = [{ row: 'Pickaxe', maxShot: 15, rangeM: 3, rate: 3, pellets: 1 }];
for (const n of names) {
  const r = rows.get(n);
  const pellets = Math.max(1, num(r, 'BulletsPerCartridge', 1));
  const cpf = Math.max(1, num(r, 'CartridgePerFire', 1));
  const crit = Math.max(1, num(r, 'DamageZone_Critical', 1));
  const dice = num(r, 'DiceCritChance') > 0 ? Math.max(1, num(r, 'DiceCritDamageMultiplier', 1)) : 1;
  const dmg = Math.max(num(r, 'DmgPB'), num(r, 'DmgMid'), num(r, 'DmgLong'), num(r, 'DmgMaxRange')) * Math.max(1, num(r, 'DmgScale', 1));
  const maxShot = Math.ceil(dmg * pellets * crit * dice * 100) / 100;
  const rate = num(r, 'FiringRate') * cpf * Math.max(1, num(r, 'ROFScale', 1));
  const rangeM = Math.max(num(r, 'RngMax'), num(r, 'RngLong'), num(r, 'MaxActualRange'), 5000) / 100;
  out.push({ row: n, maxShot, rangeM: Math.round(rangeM * 10) / 10, rate: Math.round(rate * 1000) / 1000, pellets });
}
const ts = 'export interface WeaponCap { row: string; maxShot: number; rangeM: number; rate: number; pellets: number }\nexport const WEAPONS: WeaponCap[] = [\n' + out.map((w) => '  ' + JSON.stringify(w) + ',').join('\n') + '\n];\n';
fs.writeFileSync(path.join(here, 'src/caps.ts'), ts);
const rs = 'pub struct WeaponCap { pub row: &\'static str, pub max_shot: f32, pub range_m: f32, pub rate: f32, pub pellets: u8 }\npub const WEAPONS: &[WeaponCap] = &[\n' + out.map((w) => `    WeaponCap { row: ${JSON.stringify(w.row)}, max_shot: ${w.maxShot.toFixed(2)}, range_m: ${w.rangeM.toFixed(1)}, rate: ${w.rate.toFixed(3)}, pellets: ${w.pellets} },`).join('\n') + '\n];\n';
fs.mkdirSync(path.join(repo, 'crates/fn-net/src'), { recursive: true });
fs.writeFileSync(path.join(repo, 'crates/fn-net/src/caps.rs'), rs);
console.log(out.length + ' weapons');
for (const w of out) console.log(w.row.padEnd(48), w.maxShot, w.rangeM, w.rate, w.pellets);
