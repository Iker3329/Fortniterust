import { cleanName } from './names';
import { cleanChat } from './chat';
import { WEAPONS } from './caps';
import { T, SIZE, MAX_BINARY, MAX_TEXT, MAX_PLAYERS, HOST_ONLY, SERVER_ONLY, decodeState, decodeFire, decodeHit, decodeBuild, validHostMsg, relay, correct, pong, damageMsg, elimMsg, healthMsg, spawnMsg } from './proto';

export interface Env {
  LOBBY: DurableObjectNamespace;
  LIMITER: DurableObjectNamespace;
  ALLOWED_ORIGINS: string;
}

const CODE_ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789';
const CODE_RE = /^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{6}$/;
const LOBBY_TTL_MS = 3 * 3600 * 1000;
const IDLE_LOBBY_MS = 45 * 60 * 1000;
const HELLO_TIMEOUT_MS = 10000;
const RESUME_GRACE_MS = 30000;
const KICK_STRIKES = 40;
const MAX_HEALTH = 100;
const MAX_SHIELD = 100;
const RESPAWN_MS = 5000;
const REWIND_MS = 250;
const NO_KILLER = 0xff;
const CHAT_RATE = 1;
const CHAT_BURST = 4;
const NAME_RATE = 0.2;
const NAME_BURST = 2;

function randomCode(): string {
  const b = new Uint8Array(6);
  crypto.getRandomValues(b);
  let s = '';
  for (const x of b) s += CODE_ALPHABET[x % CODE_ALPHABET.length];
  return s;
}
function randomToken(): string {
  const b = new Uint8Array(24);
  crypto.getRandomValues(b);
  return btoa(String.fromCharCode(...b)).replace(/[+/=]/g, (c) => (c === '+' ? '-' : c === '/' ? '_' : ''));
}
function originAllowed(env: Env, origin: string | null): boolean {
  if (!origin) return false;
  return env.ALLOWED_ORIGINS.split(',').map((s) => s.trim()).some((o) => o && o.toLowerCase() === origin.toLowerCase());
}
function plain(status: number, text: string): Response {
  return new Response(text, { status, headers: { 'Content-Type': 'text/plain', 'Cache-Control': 'no-store' } });
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    if (url.pathname === '/health') return plain(200, 'ok');
    if (req.headers.get('Upgrade')?.toLowerCase() !== 'websocket') return plain(426, 'websocket only');
    if (!originAllowed(env, req.headers.get('Origin'))) return plain(403, 'origin not allowed');
    if (url.pathname === '/create') {
      const ip = req.headers.get('CF-Connecting-IP') || 'local';
      const lim = env.LIMITER.get(env.LIMITER.idFromName('global'));
      const ok = await lim.fetch('https://limiter/check', { method: 'POST', body: ip });
      if (ok.status !== 200) return plain(429, 'too many lobbies');
      for (let i = 0; i < 8; i++) {
        const code = randomCode();
        const stub = env.LOBBY.get(env.LOBBY.idFromName(code));
        const h = new Headers(req.headers);
        h.set('X-MP-Op', 'create'); h.set('X-MP-Code', code);
        const r = await stub.fetch(new Request(req.url, { headers: h }));
        if (r.status === 409) continue;
        return r;
      }
      return plain(503, 'no free lobby code');
    }
    const m = url.pathname.match(/^\/join\/([A-Za-z0-9]{6})$/);
    if (m) {
      const code = m[1].toUpperCase();
      if (!CODE_RE.test(code)) return plain(404, 'bad code');
      const stub = env.LOBBY.get(env.LOBBY.idFromName(code));
      const h = new Headers(req.headers);
      h.set('X-MP-Op', 'join'); h.set('X-MP-Code', code);
      return stub.fetch(new Request(req.url, { headers: h }));
    }
    return plain(404, 'not found');
  },
};

export class Limiter {
  private hits = new Map<string, number[]>();
  constructor(_s: DurableObjectState, _e: Env) {}
  async fetch(req: Request): Promise<Response> {
    const key = await req.text();
    const now = Date.now();
    const d = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(key)));
    const k = Array.from(d.slice(0, 8)).join('.');
    const list = (this.hits.get(k) || []).filter((t) => now - t < 60000);
    if (list.length >= 6) { this.hits.set(k, list); return new Response('limited', { status: 429 }); }
    list.push(now);
    this.hits.set(k, list);
    if (this.hits.size > 5000) for (const [kk, v] of this.hits) if (!v.some((t) => now - t < 60000)) this.hits.delete(kk);
    return new Response('ok');
  }
}

class Bucket {
  tokens: number;
  last = Date.now();
  constructor(public rate: number, public burst: number) { this.tokens = burst; }
  take(n = 1): boolean {
    const now = Date.now();
    this.tokens = Math.min(this.burst, this.tokens + ((now - this.last) / 1000) * this.rate);
    this.last = now;
    if (this.tokens < n) return false;
    this.tokens -= n;
    return true;
  }
}

interface Track { t: number; x: number; y: number; z: number }

interface Player {
  slot: number;
  name: string;
  ws: WebSocket | null;
  token: string;
  host: boolean;
  joinedAt: number;
  goneAt: number;
  msgs: Bucket;
  states: Bucket;
  builds: Bucket;
  heals: Bucket;
  interacts: Bucket;
  fires: Map<number, Bucket>;
  strikes: number;
  strikeAt: number;
  track: Track[];
  last: Track | null;
  teleportOk: boolean;
  recentFires: Map<number, { t: number; weapon: number; hits: Set<number> }>;
  alive: boolean;
  health: number;
  shield: number;
  respawn: ReturnType<typeof setTimeout> | null;
}

export class Lobby {
  private code = '';
  private created = false;
  private createdAt = 0;
  private phase: 'lobby' | 'match' = 'lobby';
  private players = new Map<number, Player>();
  private pending = new Set<WebSocket>();
  private seed = 0;
  private chatBuckets = new Map<number, { token: string; chat: Bucket; name: Bucket }>();

  constructor(private state: DurableObjectState, private env: Env) {}

  async fetch(req: Request): Promise<Response> {
    const op = req.headers.get('X-MP-Op');
    const code = req.headers.get('X-MP-Code') || '';
    if (op === 'create' && this.created) return new Response('taken', { status: 409 });
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair) as [WebSocket, WebSocket];
    (server as any).binaryType = "arraybuffer";
    server.accept();
    if (op === 'create') {
      this.reset();
      this.created = true;
      this.createdAt = Date.now();
      this.code = code;
      this.seed = (Math.random() * 0xffffffff) >>> 0;
      await this.state.storage.setAlarm(Date.now() + 60000);
    }
    this.attach(server, op === 'create');
    return new Response(null, { status: 101, webSocket: client });
  }

  private reset() {
    for (const p of this.players.values()) if (p.respawn) clearTimeout(p.respawn);
    for (const p of this.players.values()) { try { p.ws?.close(4000, 'closed'); } catch (_) {} }
    for (const w of this.pending) { try { w.close(4000, 'closed'); } catch (_) {} }
    this.players.clear();
    this.pending.clear();
    this.created = false;
    this.phase = 'lobby';
    this.code = '';
  }

  private fail(ws: WebSocket, code: string, close = 4004) {
    try { ws.send(JSON.stringify({ t: 'error', code })); } catch (_) {}
    try { ws.close(close, code); } catch (_) {}
  }

  private attach(ws: WebSocket, creator: boolean) {
    this.pending.add(ws);
    let player: Player | null = null;
    const timer = setTimeout(() => { if (!player) { this.pending.delete(ws); this.fail(ws, 'hello_timeout', 4008); } }, HELLO_TIMEOUT_MS);
    ws.addEventListener('message', (ev: MessageEvent) => {
      const data = ev.data;
      if (!player) {
        if (typeof data !== 'string' || data.length > MAX_TEXT) { this.pending.delete(ws); this.fail(ws, 'bad_hello', 4002); return; }
        let msg: any;
        try { msg = JSON.parse(data); } catch (_) { this.pending.delete(ws); this.fail(ws, 'bad_hello', 4002); return; }
        clearTimeout(timer);
        this.pending.delete(ws);
        player = this.hello(ws, msg, creator);
        return;
      }
      this.onMessage(player, data);
    });
    const gone = () => { clearTimeout(timer); this.pending.delete(ws); if (player && player.ws === ws) this.left(player); };
    ws.addEventListener('close', gone);
    ws.addEventListener('error', gone);
  }

  private hello(ws: WebSocket, msg: any, creator: boolean): Player | null {
    if (!this.created) { this.fail(ws, 'no_lobby'); return null; }
    if (!msg || msg.t !== 'hello') { this.fail(ws, 'bad_hello', 4002); return null; }
    if (typeof msg.resume === 'string' && !creator) {
      for (const p of this.players.values()) {
        if (!p.ws && p.token === msg.resume && Date.now() - p.goneAt < RESUME_GRACE_MS) {
          p.ws = ws; p.goneAt = 0; p.teleportOk = true;
          ws.send(JSON.stringify({ t: 'welcome', code: this.code, slot: p.slot, host: p.host, token: p.token, phase: this.phase, seed: this.seed }));
          this.broadcastLobby();
          return p;
        }
      }
      this.fail(ws, 'resume_failed'); return null;
    }
    const name = cleanName(msg.name);
    if (!name) { this.fail(ws, 'bad_name', 4002); return null; }
    if (this.players.size >= MAX_PLAYERS) { this.fail(ws, 'full', 4009); return null; }
    let slot = 0;
    while (this.players.has(slot)) slot++;
    let finalName = name;
    const taken = new Set([...this.players.values()].map((p) => p.name.toLowerCase()));
    for (let i = 2; taken.has(finalName.toLowerCase()); i++) finalName = (name.slice(0, 13) + ' ' + i).slice(0, 16);
    const p: Player = {
      slot, name: finalName, ws, token: randomToken(), host: creator, joinedAt: Date.now(), goneAt: 0,
      msgs: new Bucket(90, 180), states: new Bucket(36, 40), builds: new Bucket(12, 24), heals: new Bucket(2, 4), interacts: new Bucket(10, 20),
      fires: new Map(), strikes: 0, strikeAt: Date.now(), track: [], last: null, teleportOk: true, recentFires: new Map(), alive: true, health: MAX_HEALTH, shield: 0, respawn: null,
    };
    this.players.set(slot, p);
    ws.send(JSON.stringify({ t: 'welcome', code: this.code, slot, host: creator, token: p.token, phase: this.phase, seed: this.seed, name: finalName }));
    this.broadcastLobby();
    return p;
  }

  private lobbyView() {
    return { t: 'lobby', code: this.code, phase: this.phase, players: [...this.players.values()].filter((p) => p.ws).map((p) => ({ slot: p.slot, name: p.name, host: p.host })) };
  }
  private broadcastLobby() { this.sendText(JSON.stringify(this.lobbyView())); }
  private sendText(s: string, except?: Player) {
    for (const p of this.players.values()) if (p !== except && p.ws) { try { p.ws.send(s); } catch (_) {} }
  }
  private sendBin(b: Uint8Array, except?: Player) {
    for (const p of this.players.values()) if (p !== except && p.ws) { try { p.ws.send(b); } catch (_) {} }
  }

  private left(p: Player) {
    p.ws = null;
    if (p.host) { this.end('host_left'); return; }
    if (this.phase === 'match') { p.goneAt = Date.now(); this.state.storage.setAlarm(Date.now() + RESUME_GRACE_MS + 1000); }
    else this.players.delete(p.slot);
    this.sendText(JSON.stringify({ t: 'left', slot: p.slot }));
    this.broadcastLobby();
    this.checkEmpty();
  }

  private checkEmpty() {
    const live = [...this.players.values()].some((p) => p.ws);
    if (!live) this.reset();
  }

  private end(reason: string) {
    const s = JSON.stringify({ t: 'end', reason });
    for (const p of this.players.values()) if (p.ws) { try { p.ws.send(s); p.ws.close(4001, reason); } catch (_) {} }
    this.reset();
  }

  async alarm() {
    if (!this.created) return;
    const now = Date.now();
    if (now - this.createdAt > LOBBY_TTL_MS) { this.end('ttl'); return; }
    if (this.phase === 'lobby' && now - this.createdAt > IDLE_LOBBY_MS) { this.end('idle'); return; }
    for (const p of [...this.players.values()]) {
      if (!p.ws && p.goneAt && now - p.goneAt > RESUME_GRACE_MS) { this.players.delete(p.slot); this.broadcastLobby(); }
    }
    this.checkEmpty();
    if (this.created) await this.state.storage.setAlarm(now + 60000);
  }

  private strike(p: Player, n: number, why: string) {
    const now = Date.now();
    p.strikes = Math.max(0, p.strikes - (now - p.strikeAt) / 1000) + n;
    p.strikeAt = now;
    if (p.strikes >= KICK_STRIKES && p.ws) {
      const ws = p.ws;
      try { ws.send(JSON.stringify({ t: 'kicked', reason: why })); } catch (_) {}
      if (p.host) { this.end('host_kicked'); return; }
      this.players.delete(p.slot);
      try { ws.close(4003, 'kicked'); } catch (_) {}
      this.sendText(JSON.stringify({ t: 'left', slot: p.slot, kicked: true }));
      this.broadcastLobby();
      this.checkEmpty();
    }
  }

  private onMessage(p: Player, data: string | ArrayBuffer) {
    if (typeof data !== "string" && typeof (data as any).arrayBuffer === "function" && !(data instanceof ArrayBuffer)) { (data as any).arrayBuffer().then((b: ArrayBuffer) => this.onMessage(p, b)); return; }
    if (!p.msgs.take()) { this.strike(p, 1, 'rate'); return; }
    if (typeof data === 'string') {
      if (data.length > MAX_TEXT) { this.strike(p, 5, 'size'); return; }
      let m: any;
      try { m = JSON.parse(data); } catch (_) { this.strike(p, 3, 'schema'); return; }
      this.onControl(p, m);
      return;
    }
    const raw: any = data;
    const len = raw.byteLength ?? 0;
    if (len > MAX_BINARY || len < 1) { this.strike(p, 5, 'size'); return; }
    const bytes = ArrayBuffer.isView(raw) ? new Uint8Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength)) : new Uint8Array(raw.slice(0));
    const v = new DataView(bytes.buffer);
    const type = bytes[0];
    if (SERVER_ONLY.has(type)) { this.strike(p, 10, 'permission'); return; }
    if (HOST_ONLY.has(type)) {
      if (!p.host) { this.strike(p, 10, 'permission'); return; }
      if (!validHostMsg(type, v)) { this.strike(p, 3, 'schema'); return; }
      this.sendBin(relay(type, p.slot, bytes), p);
      return;
    }
    switch (type) {
      case T.STATE: return this.onState(p, v, bytes);
      case T.FIRE: return this.onFire(p, v, bytes);
      case T.HIT: return this.onHit(p, v, bytes);
      case T.BUILD: {
        const b = decodeBuild(v);
        if (!b) { this.strike(p, 3, 'schema'); return; }
        if (!p.builds.take()) { this.strike(p, 2, 'build_rate'); return; }
        this.sendBin(relay(type, p.slot, bytes), p);
        return;
      }
      case T.HEAL: case T.SELFDMG: {
        if (v.byteLength !== SIZE.HEAL || bytes[1] > 100 || bytes[2] > 100) { this.strike(p, 3, 'schema'); return; }
        if (!p.heals.take()) { this.strike(p, 2, 'heal_rate'); return; }
        this.onVitals(p, bytes[1], bytes[2], type === T.HEAL);
        return;
      }
      case T.INTERACT: {
        if (v.byteLength !== SIZE.INTERACT) { this.strike(p, 3, 'schema'); return; }
        if (!p.interacts.take()) { this.strike(p, 1, 'interact_rate'); return; }
        this.sendBin(relay(type, p.slot, bytes), p);
        return;
      }
      case T.PING: {
        if (v.byteLength !== SIZE.PING) { this.strike(p, 3, 'schema'); return; }
        try { p.ws?.send(pong(v.getUint32(1, true), Date.now())); } catch (_) {}
        return;
      }
    }
    this.strike(p, 3, 'unknown_type');
  }


  private onControl(p: Player, m: any) {
    if (!m || typeof m.t !== 'string') { this.strike(p, 3, 'schema'); return; }
    switch (m.t) {
      case 'start':
        if (!p.host) { this.strike(p, 10, 'permission'); return; }
        if (this.phase === 'match') return;
        this.phase = 'match';
        for (const q of this.players.values()) { if (q.respawn) clearTimeout(q.respawn); q.respawn = null; q.alive = true; q.health = MAX_HEALTH; q.shield = 0; q.teleportOk = true; q.recentFires.clear(); }
        this.sendText(JSON.stringify({ t: 'start', seed: this.seed, at: Date.now() }));
        this.broadcastLobby();
        this.broadcastHealth();
        return;
      case 'kick': {
        if (!p.host) { this.strike(p, 10, 'permission'); return; }
        const t = this.players.get(m.slot);
        if (!t || t.host) return;
        this.players.delete(t.slot);
        try { t.ws?.send(JSON.stringify({ t: 'kicked', reason: 'host' })); t.ws?.close(4003, 'kicked'); } catch (_) {}
        this.sendText(JSON.stringify({ t: 'left', slot: t.slot, kicked: true }));
        this.broadcastLobby();
        return;
      }
      case 'end':
        if (!p.host) { this.strike(p, 10, 'permission'); return; }
        this.end('host_ended');
        return;
      case 'leave':
        try { p.ws?.close(1000, 'leave'); } catch (_) {}
        p.goneAt = 0;
        if (!p.host) this.players.delete(p.slot);
        this.left(p);
        return;
      case 'lobby':
        try { p.ws?.send(JSON.stringify(this.lobbyView())); } catch (_) {}
        return;
      case 'chat': return this.onChat(p, m.text);
      case 'name': return this.onRename(p, m.name);
    }
    this.strike(p, 3, 'unknown_control');
  }

  private talk(p: Player) {
    let b = this.chatBuckets.get(p.slot);
    if (!b || b.token !== p.token) { b = { token: p.token, chat: new Bucket(CHAT_RATE, CHAT_BURST), name: new Bucket(NAME_RATE, NAME_BURST) }; this.chatBuckets.set(p.slot, b); }
    return b;
  }
  private reject(p: Player, code: string) { try { p.ws?.send(JSON.stringify({ t: 'chat_reject', code })); } catch (_) {} }

  private onChat(p: Player, raw: unknown) {
    if (!this.talk(p).chat.take()) { this.reject(p, 'rate'); return; }
    const r = cleanChat(raw);
    if (!r.ok) { this.reject(p, r.code); return; }
    this.sendText(JSON.stringify({ t: 'chat', slot: p.slot, name: p.name, text: r.text }));
  }

  private onRename(p: Player, raw: unknown) {
    if (!this.talk(p).name.take()) { this.reject(p, 'rate'); return; }
    const name = cleanName(raw);
    if (!name) { this.reject(p, 'bad_name'); return; }
    if (name === p.name) return;
    const taken = new Set([...this.players.values()].filter((q) => q !== p).map((q) => q.name.toLowerCase()));
    let finalName = name;
    for (let i = 2; taken.has(finalName.toLowerCase()); i++) finalName = (name.slice(0, 13) + ' ' + i).slice(0, 16);
    const old = p.name;
    p.name = finalName;
    this.sendText(JSON.stringify({ t: 'renamed', slot: p.slot, old, name: finalName }));
    this.broadcastLobby();
  }

  private onState(p: Player, v: DataView, bytes: Uint8Array) {
    const s = decodeState(v);
    if (!s) { this.strike(p, 3, 'schema'); return; }
    if (!p.states.take()) { this.strike(p, 1, 'state_rate'); return; }
    const now = Date.now();
    const cur: Track = { t: now, x: s.x, y: s.y, z: s.z };
    if (p.teleportOk || !p.last) {
      p.teleportOk = false;
      p.track = [cur];
      p.last = cur;
    } else {
      const bad = this.speedViolation(p, cur);
      if (bad) {
        this.strike(p, 4, bad);
        try { p.ws?.send(correct(p.last.x, p.last.y, p.last.z)); } catch (_) {}
        return;
      }
      p.last = cur;
      p.track.push(cur);
      while (p.track.length > 2 && now - p.track[1].t > 2000) p.track.shift();
    }
    this.sendBin(relay(T.STATE, p.slot, bytes), p);
  }

  private speedViolation(p: Player, c: Track): string | null {
    const H = 8.0, UP = 12.0, DOWN = 45.0;
    const check = (a: Track, slackH: number, slackV: number): boolean => {
      const dt = Math.max(0, (c.t - a.t) / 1000);
      const dh = Math.hypot(c.x - a.x, c.z - a.z);
      const dy = c.y - a.y;
      return dh > H * dt + slackH || dy > UP * dt + slackV || -dy > DOWN * dt + slackV;
    };
    if (p.last && check(p.last, 6, 8)) return 'teleport';
    const old = p.track.find((a) => c.t - a.t >= 1000) || p.track[0];
    if (old && check(old, 2.5, 4)) return 'speed';
    return null;
  }

  private onFire(p: Player, v: DataView, bytes: Uint8Array) {
    const f = decodeFire(v);
    if (!f) { this.strike(p, 3, 'schema'); return; }
    if (!p.alive || this.phase !== 'match') return;
    const w = WEAPONS[f.weapon];
    if (!w) { this.strike(p, 3, 'weapon'); return; }
    let b = p.fires.get(f.weapon);
    if (!b) { b = new Bucket(w.rate * 1.25 + 0.2, Math.max(3, Math.ceil(w.rate * 0.6))); p.fires.set(f.weapon, b); }
    if (!b.take()) { this.strike(p, 2, 'fire_rate'); return; }
    const now = Date.now();
    p.recentFires.set(f.seq, { t: now, weapon: f.weapon, hits: new Set() });
    if (p.recentFires.size > 64) for (const [k, r] of p.recentFires) if (now - r.t > 4000) p.recentFires.delete(k);
    this.sendBin(relay(T.FIRE, p.slot, bytes), p);
  }

  private onHit(p: Player, v: DataView, bytes: Uint8Array) {
    const h = decodeHit(v);
    if (!h) { this.strike(p, 3, 'schema'); return; }
    const w = WEAPONS[h.weapon];
    const fire = p.recentFires.get(h.fireSeq);
    const target = this.players.get(h.target);
    if (!p.alive || this.phase !== 'match') return;
    if (!w || !fire || fire.weapon !== h.weapon || Date.now() - fire.t > 4000) { this.strike(p, 3, 'hit_no_fire'); return; }
    if (!target || target === p || !target.alive || !target.last || !p.last) return;
    if (fire.hits.has(h.target)) { this.strike(p, 3, 'hit_dup'); return; }
    if (h.pellets > w.pellets || h.damage > w.maxShot * (h.pellets / w.pellets) + 0.01) { this.strike(p, 8, 'damage_cap'); return; }
    if (this.rewoundDistance(p, target) > w.rangeM + 12) { this.strike(p, 6, 'range'); return; }
    fire.hits.add(h.target);
    this.damage(target, p.slot, Math.min(h.damage, w.maxShot), (h.flags & 1) !== 0, h.weapon);
  }

  private rewoundDistance(p: Player, target: Player): number {
    const now = Date.now();
    const from = p.last!;
    let best = Math.hypot(target.last!.x - from.x, target.last!.y - from.y, target.last!.z - from.z);
    for (const a of target.track) if (now - a.t <= REWIND_MS) best = Math.min(best, Math.hypot(a.x - from.x, a.y - from.y, a.z - from.z));
    return best;
  }

  private broadcastHealth() {
    this.sendBin(healthMsg([...this.players.values()].map((q) => ({ slot: q.slot, health: q.health, shield: q.shield, dead: !q.alive }))));
  }

  private damage(t: Player, attacker: number, amount: number, head: boolean, weapon: number) {
    if (!t.alive || !(amount > 0)) return;
    const fromShield = Math.min(t.shield, amount);
    t.shield -= fromShield;
    const fromHealth = Math.min(t.health, amount - fromShield);
    t.health -= fromHealth;
    const hf = head ? 2 : 0;
    if (fromShield > 0) this.sendBin(damageMsg(t.slot, attacker, 1 | hf, fromShield));
    if (fromHealth > 0) this.sendBin(damageMsg(t.slot, attacker, hf, fromHealth));
    if (t.health <= 0) this.eliminate(t, attacker, weapon, head);
    else this.broadcastHealth();
  }

  private eliminate(t: Player, killer: number, weapon: number, head: boolean) {
    t.alive = false; t.health = 0; t.shield = 0;
    this.sendBin(elimMsg(t.slot, killer, weapon, head));
    this.broadcastHealth();
    if (t.respawn) clearTimeout(t.respawn);
    t.respawn = setTimeout(() => {
      t.respawn = null;
      if (this.players.get(t.slot) !== t || this.phase !== 'match') return;
      t.alive = true; t.health = MAX_HEALTH; t.shield = 0; t.teleportOk = true;
      this.sendBin(spawnMsg(t.slot, 0, 0, 0));
      this.broadcastHealth();
    }, RESPAWN_MS);
  }

  private onVitals(p: Player, health: number, shield: number, heal: boolean) {
    if (!p.alive || this.phase !== 'match') return;
    if (heal) {
      p.health = Math.max(p.health, Math.min(MAX_HEALTH, health));
      p.shield = Math.max(p.shield, Math.min(MAX_SHIELD, shield));
    } else {
      p.health = Math.min(p.health, health);
      p.shield = Math.min(p.shield, shield);
      if (p.health <= 0) { this.eliminate(p, NO_KILLER, 0, false); return; }
    }
    this.broadcastHealth();
  }
}
