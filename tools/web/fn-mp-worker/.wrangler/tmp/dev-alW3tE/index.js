var __defProp = Object.defineProperty;
var __name = (target, value) => __defProp(target, "name", { value, configurable: true });

// src/names.ts
var SUBSTR = [
  "fuck",
  "shit",
  "cunt",
  "bitch",
  "whore",
  "slut",
  "pussy",
  "penis",
  "vagina",
  "porn",
  "rape",
  "niger",
  "niga",
  "nigr",
  "faget",
  "fagot",
  "retard",
  "trany",
  "kike",
  "chink",
  "wetback",
  "beaner",
  "raghead",
  "towelhead",
  "hitler",
  "nazi",
  "pedophil",
  "molest",
  "jizz",
  "twat",
  "wank",
  "asshole",
  "killyourself"
];
var WORD = ["fuk", "fck", "fag", "fags", "dick", "cock", "cum", "anal", "anus", "spic", "coon", "gok", "dyke", "paki", "kkk", "heil", "isis", "kys", "rapist", "jihad", "bastard", "pedo", "pedos"];
var LEET = { "0": "o", "1": "i", "3": "e", "4": "a", "5": "s", "7": "t", "8": "b", "9": "g", "$": "s", "@": "a", "!": "i", "|": "i" };
var CHARSET = /^[A-Za-z0-9 _.\-]+$/;
function fold(s) {
  let out = "";
  for (const ch of s.toLowerCase()) {
    const c = LEET[ch] ?? ch;
    out += c >= "a" && c <= "z" ? c : " ";
  }
  return out.replace(/([a-z])\1+/g, "$1");
}
__name(fold, "fold");
var dedupe = /* @__PURE__ */ __name((w) => w.replace(/([a-z])\1+/g, "$1"), "dedupe");
function cleanName(raw) {
  if (typeof raw !== "string") return null;
  const name = raw.normalize("NFKC").trim().replace(/\s+/g, " ");
  if (name.length < 3 || name.length > 16 || !CHARSET.test(name)) return null;
  if (!/[A-Za-z0-9]/.test(name)) return null;
  const words = fold(name).split(" ").filter(Boolean);
  const joined = words.join("");
  for (const w of SUBSTR) if (joined.includes(dedupe(w))) return null;
  for (const w of WORD) {
    const d = dedupe(w);
    if (words.includes(d) || joined === d) return null;
  }
  return name;
}
__name(cleanName, "cleanName");

// src/caps.ts
var WEAPONS = [
  { "row": "Pickaxe", "maxShot": 40, "rangeM": 4, "rate": 3, "pellets": 1 },
  { "row": "Assault_Auto_Athena_C_Ore_T03", "maxShot": 80, "rangeM": 250, "rate": 5.5, "pellets": 1 },
  { "row": "Assault_Auto_Athena_R_Ore_T03", "maxShot": 87.5, "rangeM": 250, "rate": 5.5, "pellets": 1 },
  { "row": "Assault_Auto_Athena_SR_Ore_T03", "maxShot": 97.5, "rangeM": 250, "rate": 5.5, "pellets": 1 },
  { "row": "Assault_Auto_Athena_UC_Ore_T03", "maxShot": 82.5, "rangeM": 250, "rate": 5.5, "pellets": 1 },
  { "row": "Assault_Auto_Athena_VR_Ore_T03", "maxShot": 92.5, "rangeM": 250, "rate": 5.5, "pellets": 1 },
  { "row": "Assault_Hydraulic_Drum_Athena_SR_Ore_T03", "maxShot": 97.5, "rangeM": 250, "rate": 5.5, "pellets": 1 },
  { "row": "Assault_SemiAuto_Athena_C_Ore_T03", "maxShot": 67.5, "rangeM": 250, "rate": 5.25, "pellets": 1 },
  { "row": "Assault_SemiAuto_Athena_R_Ore_T03", "maxShot": 75, "rangeM": 250, "rate": 5.25, "pellets": 1 },
  { "row": "Assault_SemiAuto_Athena_UC_Ore_T03", "maxShot": 72.5, "rangeM": 250, "rate": 5.25, "pellets": 1 },
  { "row": "Assault_SingleShot_Athena_VR_Ore_T03", "maxShot": 185, "rangeM": 250, "rate": 1.5, "pellets": 10 },
  { "row": "Assault_Surgical_Athena_R_Ore_T03", "maxShot": 57.5, "rangeM": 250, "rate": 3.5, "pellets": 1 },
  { "row": "Assault_Surgical_Athena_VR_Ore_T03", "maxShot": 60, "rangeM": 250, "rate": 3.5, "pellets": 1 },
  { "row": "Launcher_Grenade_Athena_R_Ore_T03", "maxShot": 100, "rangeM": 250, "rate": 1, "pellets": 1 },
  { "row": "Launcher_Grenade_Athena_SR_Ore_T03", "maxShot": 110, "rangeM": 250, "rate": 1, "pellets": 1 },
  { "row": "Launcher_Grenade_Athena_VR_Ore_T03", "maxShot": 105, "rangeM": 250, "rate": 1, "pellets": 1 },
  { "row": "Launcher_Rocket_Athena_R_Ore_T03", "maxShot": 110, "rangeM": 250, "rate": 0.75, "pellets": 1 },
  { "row": "Launcher_Rocket_Athena_SR_Ore_T03", "maxShot": 121, "rangeM": 250, "rate": 0.75, "pellets": 1 },
  { "row": "Launcher_Rocket_Athena_VR_Ore_T03", "maxShot": 116, "rangeM": 250, "rate": 0.75, "pellets": 1 },
  { "row": "Pistol_AutoHeavy_Athena_C_Ore_T03", "maxShot": 30, "rangeM": 250, "rate": 15, "pellets": 1 },
  { "row": "Pistol_AutoHeavy_Athena_R_Ore_T03", "maxShot": 32.5, "rangeM": 250, "rate": 15, "pellets": 1 },
  { "row": "Pistol_AutoHeavy_Athena_UC_Ore_T03", "maxShot": 30, "rangeM": 250, "rate": 15, "pellets": 1 },
  { "row": "Pistol_Gatling_Athena_SR_Ore_T03", "maxShot": 45, "rangeM": 250, "rate": 10, "pellets": 1 },
  { "row": "Pistol_Scavenger_Athena_R_Ore_T03", "maxShot": 37.5, "rangeM": 250, "rate": 10, "pellets": 1 },
  { "row": "Pistol_Scavenger_Athena_UC_Ore_T03", "maxShot": 35, "rangeM": 250, "rate": 10, "pellets": 1 },
  { "row": "Pistol_Scavenger_Athena_VR_Ore_T03", "maxShot": 40, "rangeM": 250, "rate": 10, "pellets": 1 },
  { "row": "Pistol_SemiAuto_Athena_C_Ore_T03", "maxShot": 57.5, "rangeM": 250, "rate": 6.75, "pellets": 1 },
  { "row": "Pistol_SemiAuto_Athena_R_Ore_T03", "maxShot": 62.5, "rangeM": 250, "rate": 6.75, "pellets": 1 },
  { "row": "Pistol_SemiAuto_Athena_UC_Ore_T03", "maxShot": 60, "rangeM": 250, "rate": 6.75, "pellets": 1 },
  { "row": "Pistol_SixShooter_Athena_C_Ore_T03", "maxShot": 135, "rangeM": 250, "rate": 0.9, "pellets": 1 },
  { "row": "Pistol_SixShooter_Athena_R_Ore_T03", "maxShot": 150, "rangeM": 250, "rate": 0.9, "pellets": 1 },
  { "row": "Pistol_SixShooter_Athena_UC_Ore_T03", "maxShot": 142.5, "rangeM": 250, "rate": 0.9, "pellets": 1 },
  { "row": "Pistol_Zapper_Athena_SR_Ore_T03", "maxShot": 121, "rangeM": 250, "rate": 0.75, "pellets": 1 },
  { "row": "Shotgun_SemiAuto_Athena_R_Ore_T03", "maxShot": 175, "rangeM": 250, "rate": 1.5, "pellets": 10 },
  { "row": "Shotgun_SemiAuto_Athena_UC_Ore_T03", "maxShot": 167.5, "rangeM": 250, "rate": 1.5, "pellets": 10 },
  { "row": "Shotgun_SemiAuto_Athena_VR_Ore_T03", "maxShot": 185, "rangeM": 250, "rate": 1.5, "pellets": 10 },
  { "row": "Shotgun_Standard_Athena_C_Ore_T03", "maxShot": 225, "rangeM": 250, "rate": 0.7, "pellets": 10 },
  { "row": "Shotgun_Standard_Athena_UC_Ore_T03", "maxShot": 237.5, "rangeM": 250, "rate": 0.7, "pellets": 10 },
  { "row": "Sniper_AMR_Athena_SR_Ore_T03", "maxShot": 125, "rangeM": 250, "rate": 1, "pellets": 1 },
  { "row": "Sniper_BoltAction_Scope_Athena_R_Ore_T03", "maxShot": 262.5, "rangeM": 250, "rate": 0.33, "pellets": 1 },
  { "row": "Sniper_BoltAction_Scope_Athena_SR_Ore_T03", "maxShot": 290, "rangeM": 250, "rate": 0.33, "pellets": 1 },
  { "row": "Sniper_BoltAction_Scope_Athena_VR_Ore_T03", "maxShot": 275, "rangeM": 250, "rate": 0.33, "pellets": 1 },
  { "row": "Sniper_Shredder_Athena_SR_Ore_T03", "maxShot": 275, "rangeM": 250, "rate": 0.7, "pellets": 10 },
  { "row": "Sniper_Standard_Scope_Athena_SR_Ore_T03", "maxShot": 165, "rangeM": 250, "rate": 1.2, "pellets": 1 },
  { "row": "Sniper_Standard_Scope_Athena_VR_Ore_T03", "maxShot": 157.5, "rangeM": 250, "rate": 1.2, "pellets": 1 },
  { "row": "Sniper_TripleShot_Athena_SR_Ore_T03", "maxShot": 290, "rangeM": 250, "rate": 0.33, "pellets": 1 }
];

// src/proto.ts
var T = {
  STATE: 1,
  FIRE: 2,
  HIT: 3,
  BUILD: 4,
  HEAL: 5,
  SELFDMG: 6,
  INTERACT: 7,
  PING: 9,
  HEALTH: 64,
  ELIM: 65,
  STORM: 66,
  SPAWN: 67,
  LOOT: 68,
  DAMAGE: 69,
  CORRECT: 128,
  PONG: 137
};
var SERVER_SLOT = 255;
var MAX_BINARY = 256;
var MAX_TEXT = 512;
var MAX_PLAYERS = 16;
var SIZE = {
  STATE: 45,
  FIRE: 31,
  HIT: 13,
  BUILD_HEAD: 36,
  HEAL: 3,
  SELFDMG: 3,
  INTERACT: 6,
  PING: 5,
  ELIM: 5,
  STORM: 32,
  SPAWN: 14,
  DAMAGE: 8
};
var HOST_ONLY = /* @__PURE__ */ new Set([T.HEALTH, T.ELIM, T.STORM, T.SPAWN, T.LOOT, T.DAMAGE]);
var TO_HOST = /* @__PURE__ */ new Set([T.HIT, T.HEAL, T.SELFDMG]);
var finite = /* @__PURE__ */ __name((...v) => v.every((x) => Number.isFinite(x)), "finite");
function decodeState(b) {
  if (b.byteLength !== SIZE.STATE) return null;
  const s = {
    seq: b.getUint32(1, true),
    tms: b.getUint32(5, true),
    x: b.getFloat32(9, true),
    y: b.getFloat32(13, true),
    z: b.getFloat32(17, true),
    vx: b.getFloat32(21, true),
    vy: b.getFloat32(25, true),
    vz: b.getFloat32(29, true),
    yaw: b.getFloat32(33, true),
    pitch: b.getFloat32(37, true),
    flags: b.getUint16(41, true),
    held: b.getUint8(43),
    extra: b.getUint8(44)
  };
  if (!finite(s.x, s.y, s.z, s.vx, s.vy, s.vz, s.yaw, s.pitch)) return null;
  if (Math.abs(s.x) > 6e3 || Math.abs(s.z) > 6e3 || s.y < -200 || s.y > 3e3) return null;
  if (Math.hypot(s.vx, s.vz) > 60 || Math.abs(s.vy) > 60) return null;
  if (Math.abs(s.pitch) > 100 || Math.abs(s.yaw) > 1e3) return null;
  return s;
}
__name(decodeState, "decodeState");
function decodeFire(b) {
  if (b.byteLength !== SIZE.FIRE) return null;
  const f = {
    seq: b.getUint32(1, true),
    weapon: b.getUint8(5),
    flags: b.getUint8(6),
    ox: b.getFloat32(7, true),
    oy: b.getFloat32(11, true),
    oz: b.getFloat32(15, true),
    dx: b.getFloat32(19, true),
    dy: b.getFloat32(23, true),
    dz: b.getFloat32(27, true)
  };
  if (!finite(f.ox, f.oy, f.oz, f.dx, f.dy, f.dz)) return null;
  const l = Math.hypot(f.dx, f.dy, f.dz);
  if (l < 0.9 || l > 1.1) return null;
  return f;
}
__name(decodeFire, "decodeFire");
function decodeHit(b) {
  if (b.byteLength !== SIZE.HIT) return null;
  const h = { fireSeq: b.getUint32(1, true), target: b.getUint8(5), weapon: b.getUint8(6), flags: b.getUint8(7), pellets: b.getUint8(8), damage: b.getFloat32(9, true) };
  if (!finite(h.damage) || h.damage <= 0 || h.pellets < 1) return null;
  return h;
}
__name(decodeHit, "decodeHit");
var CLASS_RE = /^\/Game\/[A-Za-z0-9_\/.]{1,160}$/;
function decodeBuild(b) {
  if (b.byteLength < SIZE.BUILD_HEAD) return null;
  const len = b.getUint8(35);
  if (b.byteLength !== SIZE.BUILD_HEAD + len) return null;
  const op = b.getUint8(1);
  if (op < 1 || op > 3) return null;
  const bytes = new Uint8Array(b.buffer, b.byteOffset + SIZE.BUILD_HEAD, len);
  let classPath = "";
  for (const c of bytes) classPath += String.fromCharCode(c);
  if (len > 0 && !CLASS_RE.test(classPath)) return null;
  const r = {
    op,
    kind: b.getUint8(2),
    material: b.getUint8(3),
    rot: b.getUint8(4),
    cell: [b.getInt32(5, true), b.getInt32(9, true), b.getInt32(13, true)],
    loc: [b.getFloat32(17, true), b.getFloat32(21, true), b.getFloat32(25, true)],
    yaw: b.getFloat32(29, true),
    rotIter: b.getInt8(33),
    mirrored: b.getUint8(34),
    classPath
  };
  if (!finite(...r.loc, r.yaw) || r.rot > 3 || r.material > 2 || r.kind > 7 || r.mirrored > 1) return null;
  if (r.cell.some((c) => Math.abs(c) > 1e5) || r.loc.some((c) => Math.abs(c) > 6e5)) return null;
  return r;
}
__name(decodeBuild, "decodeBuild");
function validHostMsg(type, b) {
  switch (type) {
    case T.HEALTH: {
      if (b.byteLength < 2) return false;
      const n = b.getUint8(1);
      if (n > MAX_PLAYERS || b.byteLength !== 2 + 4 * n) return false;
      for (let i = 0; i < n; i++) {
        if (b.getUint8(2 + i * 4 + 1) > 100 || b.getUint8(2 + i * 4 + 2) > 100) return false;
      }
      return true;
    }
    case T.ELIM:
      return b.byteLength === SIZE.ELIM;
    case T.STORM: {
      if (b.byteLength !== SIZE.STORM) return false;
      for (let o = 4; o < 32; o += 4) if (!Number.isFinite(b.getFloat32(o, true))) return false;
      return true;
    }
    case T.SPAWN:
      return b.byteLength === SIZE.SPAWN && Number.isFinite(b.getFloat32(2, true)) && Number.isFinite(b.getFloat32(6, true)) && Number.isFinite(b.getFloat32(10, true));
    case T.LOOT:
      return b.byteLength >= 6 && b.byteLength <= 200;
    case T.DAMAGE:
      return b.byteLength === SIZE.DAMAGE && Number.isFinite(b.getFloat32(4, true)) && b.getFloat32(4, true) >= 0 && b.getFloat32(4, true) <= 1e3;
  }
  return false;
}
__name(validHostMsg, "validHostMsg");
function relay(type, from, payload) {
  const out = new Uint8Array(payload.byteLength + 1);
  out[0] = type;
  out[1] = from;
  out.set(payload.subarray(1), 2);
  return out;
}
__name(relay, "relay");
function correct(x, y, z) {
  const out = new Uint8Array(14);
  const v = new DataView(out.buffer);
  out[0] = T.CORRECT;
  out[1] = SERVER_SLOT;
  v.setFloat32(2, x, true);
  v.setFloat32(6, y, true);
  v.setFloat32(10, z, true);
  return out;
}
__name(correct, "correct");
function pong(nonce, nowMs) {
  const out = new Uint8Array(14);
  const v = new DataView(out.buffer);
  out[0] = T.PONG;
  out[1] = SERVER_SLOT;
  v.setUint32(2, nonce, true);
  v.setFloat64(6, nowMs, true);
  return out;
}
__name(pong, "pong");

// src/index.ts
var CODE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
var CODE_RE = /^[ABCDEFGHJKMNPQRSTUVWXYZ23456789]{6}$/;
var LOBBY_TTL_MS = 3 * 3600 * 1e3;
var IDLE_LOBBY_MS = 45 * 60 * 1e3;
var HELLO_TIMEOUT_MS = 1e4;
var RESUME_GRACE_MS = 3e4;
var KICK_STRIKES = 40;
function randomCode() {
  const b = new Uint8Array(6);
  crypto.getRandomValues(b);
  let s = "";
  for (const x of b) s += CODE_ALPHABET[x % CODE_ALPHABET.length];
  return s;
}
__name(randomCode, "randomCode");
function randomToken() {
  const b = new Uint8Array(24);
  crypto.getRandomValues(b);
  return btoa(String.fromCharCode(...b)).replace(/[+/=]/g, (c) => c === "+" ? "-" : c === "/" ? "_" : "");
}
__name(randomToken, "randomToken");
function originAllowed(env, origin) {
  if (!origin) return false;
  return env.ALLOWED_ORIGINS.split(",").map((s) => s.trim()).some((o) => o && o.toLowerCase() === origin.toLowerCase());
}
__name(originAllowed, "originAllowed");
function plain(status, text) {
  return new Response(text, { status, headers: { "Content-Type": "text/plain", "Cache-Control": "no-store" } });
}
__name(plain, "plain");
var src_default = {
  async fetch(req, env) {
    const url = new URL(req.url);
    if (url.pathname === "/health") return plain(200, "ok");
    if (req.headers.get("Upgrade")?.toLowerCase() !== "websocket") return plain(426, "websocket only");
    if (!originAllowed(env, req.headers.get("Origin"))) return plain(403, "origin not allowed");
    if (url.pathname === "/create") {
      const ip = req.headers.get("CF-Connecting-IP") || "local";
      const lim = env.LIMITER.get(env.LIMITER.idFromName("global"));
      const ok = await lim.fetch("https://limiter/check", { method: "POST", body: ip });
      if (ok.status !== 200) return plain(429, "too many lobbies");
      for (let i = 0; i < 8; i++) {
        const code = randomCode();
        const stub = env.LOBBY.get(env.LOBBY.idFromName(code));
        const h = new Headers(req.headers);
        h.set("X-MP-Op", "create");
        h.set("X-MP-Code", code);
        const r = await stub.fetch(new Request(req.url, { headers: h }));
        if (r.status === 409) continue;
        return r;
      }
      return plain(503, "no free lobby code");
    }
    const m = url.pathname.match(/^\/join\/([A-Za-z0-9]{6})$/);
    if (m) {
      const code = m[1].toUpperCase();
      if (!CODE_RE.test(code)) return plain(404, "bad code");
      const stub = env.LOBBY.get(env.LOBBY.idFromName(code));
      const h = new Headers(req.headers);
      h.set("X-MP-Op", "join");
      h.set("X-MP-Code", code);
      return stub.fetch(new Request(req.url, { headers: h }));
    }
    return plain(404, "not found");
  }
};
var Limiter = class {
  static {
    __name(this, "Limiter");
  }
  hits = /* @__PURE__ */ new Map();
  constructor(_s, _e) {
  }
  async fetch(req) {
    const key = await req.text();
    const now = Date.now();
    const d = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(key)));
    const k = Array.from(d.slice(0, 8)).join(".");
    const list = (this.hits.get(k) || []).filter((t) => now - t < 6e4);
    if (list.length >= 6) {
      this.hits.set(k, list);
      return new Response("limited", { status: 429 });
    }
    list.push(now);
    this.hits.set(k, list);
    if (this.hits.size > 5e3) {
      for (const [kk, v] of this.hits) if (!v.some((t) => now - t < 6e4)) this.hits.delete(kk);
    }
    return new Response("ok");
  }
};
var Bucket = class {
  constructor(rate, burst) {
    this.rate = rate;
    this.burst = burst;
    this.tokens = burst;
  }
  rate;
  burst;
  static {
    __name(this, "Bucket");
  }
  tokens;
  last = Date.now();
  take(n = 1) {
    const now = Date.now();
    this.tokens = Math.min(this.burst, this.tokens + (now - this.last) / 1e3 * this.rate);
    this.last = now;
    if (this.tokens < n) return false;
    this.tokens -= n;
    return true;
  }
};
var Lobby = class {
  constructor(state, env) {
    this.state = state;
    this.env = env;
  }
  state;
  env;
  static {
    __name(this, "Lobby");
  }
  code = "";
  created = false;
  createdAt = 0;
  phase = "lobby";
  players = /* @__PURE__ */ new Map();
  pending = /* @__PURE__ */ new Set();
  seed = 0;
  async fetch(req) {
    const op = req.headers.get("X-MP-Op");
    const code = req.headers.get("X-MP-Code") || "";
    if (op === "create" && this.created) return new Response("taken", { status: 409 });
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    server.binaryType = "arraybuffer";
    server.accept();
    if (op === "create") {
      this.reset();
      this.created = true;
      this.createdAt = Date.now();
      this.code = code;
      this.seed = Math.random() * 4294967295 >>> 0;
      await this.state.storage.setAlarm(Date.now() + 6e4);
    }
    this.attach(server, op === "create");
    return new Response(null, { status: 101, webSocket: client });
  }
  reset() {
    for (const p of this.players.values()) {
      try {
        p.ws?.close(4e3, "closed");
      } catch (_) {
      }
    }
    for (const w of this.pending) {
      try {
        w.close(4e3, "closed");
      } catch (_) {
      }
    }
    this.players.clear();
    this.pending.clear();
    this.created = false;
    this.phase = "lobby";
    this.code = "";
  }
  fail(ws, code, close = 4004) {
    try {
      ws.send(JSON.stringify({ t: "error", code }));
    } catch (_) {
    }
    try {
      ws.close(close, code);
    } catch (_) {
    }
  }
  attach(ws, creator) {
    this.pending.add(ws);
    let player = null;
    const timer = setTimeout(() => {
      if (!player) {
        this.pending.delete(ws);
        this.fail(ws, "hello_timeout", 4008);
      }
    }, HELLO_TIMEOUT_MS);
    ws.addEventListener("message", (ev) => {
      const data = ev.data;
      if (!player) {
        if (typeof data !== "string" || data.length > MAX_TEXT) {
          this.pending.delete(ws);
          this.fail(ws, "bad_hello", 4002);
          return;
        }
        let msg;
        try {
          msg = JSON.parse(data);
        } catch (_) {
          this.pending.delete(ws);
          this.fail(ws, "bad_hello", 4002);
          return;
        }
        clearTimeout(timer);
        this.pending.delete(ws);
        player = this.hello(ws, msg, creator);
        return;
      }
      this.onMessage(player, data);
    });
    const gone = /* @__PURE__ */ __name(() => {
      clearTimeout(timer);
      this.pending.delete(ws);
      if (player && player.ws === ws) this.left(player);
    }, "gone");
    ws.addEventListener("close", gone);
    ws.addEventListener("error", gone);
  }
  hello(ws, msg, creator) {
    if (!this.created) {
      this.fail(ws, "no_lobby");
      return null;
    }
    if (!msg || msg.t !== "hello") {
      this.fail(ws, "bad_hello", 4002);
      return null;
    }
    if (typeof msg.resume === "string" && !creator) {
      for (const p2 of this.players.values()) {
        if (!p2.ws && p2.token === msg.resume && Date.now() - p2.goneAt < RESUME_GRACE_MS) {
          p2.ws = ws;
          p2.goneAt = 0;
          p2.teleportOk = true;
          ws.send(JSON.stringify({ t: "welcome", code: this.code, slot: p2.slot, host: p2.host, token: p2.token, phase: this.phase, seed: this.seed }));
          this.broadcastLobby();
          return p2;
        }
      }
      this.fail(ws, "resume_failed");
      return null;
    }
    const name = cleanName(msg.name);
    if (!name) {
      this.fail(ws, "bad_name", 4002);
      return null;
    }
    if (this.players.size >= MAX_PLAYERS) {
      this.fail(ws, "full", 4009);
      return null;
    }
    let slot = 0;
    while (this.players.has(slot)) slot++;
    let finalName = name;
    const taken = new Set([...this.players.values()].map((p2) => p2.name.toLowerCase()));
    for (let i = 2; taken.has(finalName.toLowerCase()); i++) finalName = (name.slice(0, 13) + " " + i).slice(0, 16);
    const p = {
      slot,
      name: finalName,
      ws,
      token: randomToken(),
      host: creator,
      joinedAt: Date.now(),
      goneAt: 0,
      msgs: new Bucket(90, 180),
      states: new Bucket(36, 40),
      builds: new Bucket(12, 24),
      heals: new Bucket(2, 4),
      interacts: new Bucket(10, 20),
      fires: /* @__PURE__ */ new Map(),
      strikes: 0,
      strikeAt: Date.now(),
      track: [],
      last: null,
      teleportOk: true,
      recentFires: /* @__PURE__ */ new Map(),
      alive: true
    };
    this.players.set(slot, p);
    ws.send(JSON.stringify({ t: "welcome", code: this.code, slot, host: creator, token: p.token, phase: this.phase, seed: this.seed, name: finalName }));
    this.broadcastLobby();
    return p;
  }
  lobbyView() {
    return { t: "lobby", code: this.code, phase: this.phase, players: [...this.players.values()].filter((p) => p.ws).map((p) => ({ slot: p.slot, name: p.name, host: p.host })) };
  }
  broadcastLobby() {
    this.sendText(JSON.stringify(this.lobbyView()));
  }
  sendText(s, except) {
    for (const p of this.players.values()) if (p !== except && p.ws) {
      try {
        p.ws.send(s);
      } catch (_) {
      }
    }
  }
  sendBin(b, except) {
    for (const p of this.players.values()) if (p !== except && p.ws) {
      try {
        p.ws.send(b);
      } catch (_) {
      }
    }
  }
  host() {
    for (const p of this.players.values()) if (p.host) return p;
    return void 0;
  }
  left(p) {
    p.ws = null;
    if (p.host) {
      this.end("host_left");
      return;
    }
    if (this.phase === "match") {
      p.goneAt = Date.now();
      this.state.storage.setAlarm(Date.now() + RESUME_GRACE_MS + 1e3);
    } else this.players.delete(p.slot);
    this.sendText(JSON.stringify({ t: "left", slot: p.slot }));
    this.broadcastLobby();
    this.checkEmpty();
  }
  checkEmpty() {
    const live = [...this.players.values()].some((p) => p.ws);
    if (!live) this.reset();
  }
  end(reason) {
    const s = JSON.stringify({ t: "end", reason });
    for (const p of this.players.values()) if (p.ws) {
      try {
        p.ws.send(s);
        p.ws.close(4001, reason);
      } catch (_) {
      }
    }
    this.reset();
  }
  async alarm() {
    if (!this.created) return;
    const now = Date.now();
    if (now - this.createdAt > LOBBY_TTL_MS) {
      this.end("ttl");
      return;
    }
    if (this.phase === "lobby" && now - this.createdAt > IDLE_LOBBY_MS) {
      this.end("idle");
      return;
    }
    for (const p of [...this.players.values()]) {
      if (!p.ws && p.goneAt && now - p.goneAt > RESUME_GRACE_MS) {
        this.players.delete(p.slot);
        this.broadcastLobby();
      }
    }
    this.checkEmpty();
    if (this.created) await this.state.storage.setAlarm(now + 6e4);
  }
  strike(p, n, why) {
    const now = Date.now();
    p.strikes = Math.max(0, p.strikes - (now - p.strikeAt) / 1e3) + n;
    p.strikeAt = now;
    if (p.strikes >= KICK_STRIKES && p.ws) {
      const ws = p.ws;
      try {
        ws.send(JSON.stringify({ t: "kicked", reason: why }));
      } catch (_) {
      }
      if (p.host) {
        this.end("host_kicked");
        return;
      }
      this.players.delete(p.slot);
      try {
        ws.close(4003, "kicked");
      } catch (_) {
      }
      this.sendText(JSON.stringify({ t: "left", slot: p.slot, kicked: true }));
      this.broadcastLobby();
      this.checkEmpty();
    }
  }
  onMessage(p, data) {
    if (typeof data !== "string" && typeof data.arrayBuffer === "function" && !(data instanceof ArrayBuffer)) {
      data.arrayBuffer().then((b) => this.onMessage(p, b));
      return;
    }
    if (!p.msgs.take()) {
      this.strike(p, 1, "rate");
      return;
    }
    if (typeof data === "string") {
      if (data.length > MAX_TEXT) {
        this.strike(p, 5, "size");
        return;
      }
      let m;
      try {
        m = JSON.parse(data);
      } catch (_) {
        this.strike(p, 3, "schema");
        return;
      }
      this.onControl(p, m);
      return;
    }
    const raw = data;
    const len = raw.byteLength ?? 0;
    if (len > MAX_BINARY || len < 1) {
      this.strike(p, 5, "size");
      return;
    }
    const bytes = ArrayBuffer.isView(raw) ? new Uint8Array(raw.buffer.slice(raw.byteOffset, raw.byteOffset + raw.byteLength)) : new Uint8Array(raw.slice(0));
    const v = new DataView(bytes.buffer);
    const type = bytes[0];
    if (HOST_ONLY.has(type)) {
      if (!p.host) {
        this.strike(p, 10, "permission");
        return;
      }
      if (!validHostMsg(type, v)) {
        this.strike(p, 3, "schema");
        return;
      }
      if (type === T.SPAWN) {
        const t = this.players.get(bytes[1]);
        if (t) {
          t.teleportOk = true;
          t.alive = true;
        }
      }
      if (type === T.HEALTH) {
        const n = bytes[1];
        for (let i = 0; i < n; i++) {
          const t = this.players.get(bytes[2 + i * 4]);
          if (t) t.alive = (bytes[2 + i * 4 + 3] & 1) === 0;
        }
      }
      if (type === T.ELIM) {
        const t = this.players.get(bytes[1]);
        if (t) t.alive = false;
      }
      this.sendBin(relay(type, p.slot, bytes), p);
      return;
    }
    switch (type) {
      case T.STATE:
        return this.onState(p, v, bytes);
      case T.FIRE:
        return this.onFire(p, v, bytes);
      case T.HIT:
        return this.onHit(p, v, bytes);
      case T.BUILD: {
        const b = decodeBuild(v);
        if (!b) {
          this.strike(p, 3, "schema");
          return;
        }
        if (!p.builds.take()) {
          this.strike(p, 2, "build_rate");
          return;
        }
        this.sendBin(relay(type, p.slot, bytes), p);
        return;
      }
      case T.HEAL:
      case T.SELFDMG: {
        if (v.byteLength !== SIZE.HEAL || bytes[1] > 100 || bytes[2] > 100) {
          this.strike(p, 3, "schema");
          return;
        }
        if (!p.heals.take()) {
          this.strike(p, 2, "heal_rate");
          return;
        }
        this.toHost(relay(type, p.slot, bytes));
        return;
      }
      case T.INTERACT: {
        if (v.byteLength !== SIZE.INTERACT) {
          this.strike(p, 3, "schema");
          return;
        }
        if (!p.interacts.take()) {
          this.strike(p, 1, "interact_rate");
          return;
        }
        this.sendBin(relay(type, p.slot, bytes), p);
        return;
      }
      case T.PING: {
        if (v.byteLength !== SIZE.PING) {
          this.strike(p, 3, "schema");
          return;
        }
        try {
          p.ws?.send(pong(v.getUint32(1, true), Date.now()));
        } catch (_) {
        }
        return;
      }
    }
    this.strike(p, 3, "unknown_type");
  }
  toHost(b) {
    const h = this.host();
    if (h?.ws) {
      try {
        h.ws.send(b);
      } catch (_) {
      }
    }
  }
  onControl(p, m) {
    if (!m || typeof m.t !== "string") {
      this.strike(p, 3, "schema");
      return;
    }
    switch (m.t) {
      case "start":
        if (!p.host) {
          this.strike(p, 10, "permission");
          return;
        }
        if (this.phase === "match") return;
        this.phase = "match";
        this.sendText(JSON.stringify({ t: "start", seed: this.seed, at: Date.now() }));
        this.broadcastLobby();
        return;
      case "kick": {
        if (!p.host) {
          this.strike(p, 10, "permission");
          return;
        }
        const t = this.players.get(m.slot);
        if (!t || t.host) return;
        this.players.delete(t.slot);
        try {
          t.ws?.send(JSON.stringify({ t: "kicked", reason: "host" }));
          t.ws?.close(4003, "kicked");
        } catch (_) {
        }
        this.sendText(JSON.stringify({ t: "left", slot: t.slot, kicked: true }));
        this.broadcastLobby();
        return;
      }
      case "end":
        if (!p.host) {
          this.strike(p, 10, "permission");
          return;
        }
        this.end("host_ended");
        return;
      case "leave":
        try {
          p.ws?.close(1e3, "leave");
        } catch (_) {
        }
        p.goneAt = 0;
        if (!p.host) this.players.delete(p.slot);
        this.left(p);
        return;
      case "lobby":
        try {
          p.ws?.send(JSON.stringify(this.lobbyView()));
        } catch (_) {
        }
        return;
    }
    this.strike(p, 3, "unknown_control");
  }
  onState(p, v, bytes) {
    const s = decodeState(v);
    if (!s) {
      this.strike(p, 3, "schema");
      return;
    }
    if (!p.states.take()) {
      this.strike(p, 1, "state_rate");
      return;
    }
    const now = Date.now();
    const cur = { t: now, x: s.x, y: s.y, z: s.z };
    if (p.teleportOk || !p.last) {
      p.teleportOk = false;
      p.track = [cur];
      p.last = cur;
    } else {
      const bad = this.speedViolation(p, cur);
      if (bad) {
        this.strike(p, 4, bad);
        try {
          p.ws?.send(correct(p.last.x, p.last.y, p.last.z));
        } catch (_) {
        }
        return;
      }
      p.last = cur;
      p.track.push(cur);
      while (p.track.length > 2 && now - p.track[1].t > 2e3) p.track.shift();
    }
    this.sendBin(relay(T.STATE, p.slot, bytes), p);
  }
  speedViolation(p, c) {
    const H = 8, UP = 12, DOWN = 45;
    const check = /* @__PURE__ */ __name((a, slackH, slackV) => {
      const dt = Math.max(0, (c.t - a.t) / 1e3);
      const dh = Math.hypot(c.x - a.x, c.z - a.z);
      const dy = c.y - a.y;
      return dh > H * dt + slackH || dy > UP * dt + slackV || -dy > DOWN * dt + slackV;
    }, "check");
    if (p.last && check(p.last, 6, 8)) return "teleport";
    const old = p.track.find((a) => c.t - a.t >= 1e3) || p.track[0];
    if (old && check(old, 2.5, 4)) return "speed";
    return null;
  }
  onFire(p, v, bytes) {
    const f = decodeFire(v);
    if (!f) {
      this.strike(p, 3, "schema");
      return;
    }
    const w = WEAPONS[f.weapon];
    if (!w) {
      this.strike(p, 3, "weapon");
      return;
    }
    let b = p.fires.get(f.weapon);
    if (!b) {
      b = new Bucket(w.rate * 1.25 + 0.2, Math.max(3, Math.ceil(w.rate * 0.6)));
      p.fires.set(f.weapon, b);
    }
    if (!b.take()) {
      this.strike(p, 2, "fire_rate");
      return;
    }
    const now = Date.now();
    p.recentFires.set(f.seq, { t: now, weapon: f.weapon, hits: /* @__PURE__ */ new Set() });
    if (p.recentFires.size > 64) {
      for (const [k, r] of p.recentFires) if (now - r.t > 4e3) p.recentFires.delete(k);
    }
    this.sendBin(relay(T.FIRE, p.slot, bytes), p);
  }
  onHit(p, v, bytes) {
    const h = decodeHit(v);
    if (!h) {
      this.strike(p, 3, "schema");
      return;
    }
    const w = WEAPONS[h.weapon];
    const fire = p.recentFires.get(h.fireSeq);
    const target = this.players.get(h.target);
    if (!w || !fire || fire.weapon !== h.weapon || Date.now() - fire.t > 4e3) {
      this.strike(p, 3, "hit_no_fire");
      return;
    }
    if (!target || target === p || !target.alive || !target.last || !p.last) return;
    if (fire.hits.has(h.target)) {
      this.strike(p, 3, "hit_dup");
      return;
    }
    if (h.pellets > w.pellets || h.damage > w.maxShot * (h.pellets / w.pellets) + 0.01) {
      this.strike(p, 8, "damage_cap");
      return;
    }
    const dist = Math.hypot(target.last.x - p.last.x, target.last.y - p.last.y, target.last.z - p.last.z);
    if (dist > w.rangeM + 12) {
      this.strike(p, 6, "range");
      return;
    }
    fire.hits.add(h.target);
    this.toHost(relay(T.HIT, p.slot, bytes));
  }
};

// ../../../../../../AppData/Local/npm-cache/_npx/32026684e21afda6/node_modules/wrangler/templates/middleware/middleware-ensure-req-body-drained.ts
var drainBody = /* @__PURE__ */ __name(async (request, env, _ctx, middlewareCtx) => {
  try {
    return await middlewareCtx.next(request, env);
  } finally {
    try {
      if (request.body !== null && !request.bodyUsed) {
        const reader = request.body.getReader();
        while (!(await reader.read()).done) {
        }
      }
    } catch (e) {
      console.error("Failed to drain the unused request body.", e);
    }
  }
}, "drainBody");
var middleware_ensure_req_body_drained_default = drainBody;

// ../../../../../../AppData/Local/npm-cache/_npx/32026684e21afda6/node_modules/wrangler/templates/middleware/middleware-miniflare3-json-error.ts
function reduceError(e) {
  return {
    name: e?.name,
    message: e?.message ?? String(e),
    stack: e?.stack,
    cause: e?.cause === void 0 ? void 0 : reduceError(e.cause)
  };
}
__name(reduceError, "reduceError");
var jsonError = /* @__PURE__ */ __name(async (request, env, _ctx, middlewareCtx) => {
  try {
    return await middlewareCtx.next(request, env);
  } catch (e) {
    const error = reduceError(e);
    const body = JSON.stringify(error);
    const headers = {
      "Content-Type": "application/json",
      "MF-Experimental-Error-Stack": "true"
    };
    const encoded = encodeURIComponent(body);
    if (encoded.length <= 8192) {
      headers["MF-Experimental-Error-Stack-Payload"] = encoded;
    }
    return new Response(body, { status: 500, headers });
  }
}, "jsonError");
var middleware_miniflare3_json_error_default = jsonError;

// .wrangler/tmp/bundle-pcZRUc/middleware-insertion-facade.js
var __INTERNAL_WRANGLER_MIDDLEWARE__ = [
  middleware_ensure_req_body_drained_default,
  middleware_miniflare3_json_error_default
];
var middleware_insertion_facade_default = src_default;

// ../../../../../../AppData/Local/npm-cache/_npx/32026684e21afda6/node_modules/wrangler/templates/middleware/common.ts
var __facade_middleware__ = [];
function __facade_register__(...args) {
  __facade_middleware__.push(...args.flat());
}
__name(__facade_register__, "__facade_register__");
function __facade_invokeChain__(request, env, ctx, dispatch, middlewareChain) {
  const [head, ...tail] = middlewareChain;
  const middlewareCtx = {
    dispatch,
    next(newRequest, newEnv) {
      return __facade_invokeChain__(newRequest, newEnv, ctx, dispatch, tail);
    }
  };
  return head(request, env, ctx, middlewareCtx);
}
__name(__facade_invokeChain__, "__facade_invokeChain__");
function __facade_invoke__(request, env, ctx, dispatch, finalMiddleware) {
  return __facade_invokeChain__(request, env, ctx, dispatch, [
    ...__facade_middleware__,
    finalMiddleware
  ]);
}
__name(__facade_invoke__, "__facade_invoke__");

// .wrangler/tmp/bundle-pcZRUc/middleware-loader.entry.ts
var __Facade_ScheduledController__ = class ___Facade_ScheduledController__ {
  constructor(scheduledTime, cron, noRetry) {
    this.scheduledTime = scheduledTime;
    this.cron = cron;
    this.#noRetry = noRetry;
  }
  scheduledTime;
  cron;
  static {
    __name(this, "__Facade_ScheduledController__");
  }
  #noRetry;
  noRetry() {
    if (!(this instanceof ___Facade_ScheduledController__)) {
      throw new TypeError("Illegal invocation");
    }
    this.#noRetry();
  }
};
function wrapExportedHandler(worker) {
  if (__INTERNAL_WRANGLER_MIDDLEWARE__ === void 0 || __INTERNAL_WRANGLER_MIDDLEWARE__.length === 0) {
    return worker;
  }
  for (const middleware of __INTERNAL_WRANGLER_MIDDLEWARE__) {
    __facade_register__(middleware);
  }
  const fetchDispatcher = /* @__PURE__ */ __name(function(request, env, ctx) {
    if (worker.fetch === void 0) {
      throw new Error("Handler does not export a fetch() function.");
    }
    return worker.fetch(request, env, ctx);
  }, "fetchDispatcher");
  return {
    ...worker,
    fetch(request, env, ctx) {
      const dispatcher = /* @__PURE__ */ __name(function(type, init) {
        if (type === "scheduled" && worker.scheduled !== void 0) {
          const controller = new __Facade_ScheduledController__(
            Date.now(),
            init.cron ?? "",
            () => {
            }
          );
          return worker.scheduled(controller, env, ctx);
        }
      }, "dispatcher");
      return __facade_invoke__(request, env, ctx, dispatcher, fetchDispatcher);
    }
  };
}
__name(wrapExportedHandler, "wrapExportedHandler");
function wrapWorkerEntrypoint(klass) {
  if (__INTERNAL_WRANGLER_MIDDLEWARE__ === void 0 || __INTERNAL_WRANGLER_MIDDLEWARE__.length === 0) {
    return klass;
  }
  for (const middleware of __INTERNAL_WRANGLER_MIDDLEWARE__) {
    __facade_register__(middleware);
  }
  return class extends klass {
    #fetchDispatcher = /* @__PURE__ */ __name((request, env, ctx) => {
      this.env = env;
      this.ctx = ctx;
      if (super.fetch === void 0) {
        throw new Error("Entrypoint class does not define a fetch() function.");
      }
      return super.fetch(request);
    }, "#fetchDispatcher");
    #dispatcher = /* @__PURE__ */ __name((type, init) => {
      if (type === "scheduled" && super.scheduled !== void 0) {
        const controller = new __Facade_ScheduledController__(
          Date.now(),
          init.cron ?? "",
          () => {
          }
        );
        return super.scheduled(controller);
      }
    }, "#dispatcher");
    fetch(request) {
      return __facade_invoke__(
        request,
        this.env,
        this.ctx,
        this.#dispatcher,
        this.#fetchDispatcher
      );
    }
  };
}
__name(wrapWorkerEntrypoint, "wrapWorkerEntrypoint");
var WRAPPED_ENTRY;
if (typeof middleware_insertion_facade_default === "object") {
  WRAPPED_ENTRY = wrapExportedHandler(middleware_insertion_facade_default);
} else if (typeof middleware_insertion_facade_default === "function") {
  WRAPPED_ENTRY = wrapWorkerEntrypoint(middleware_insertion_facade_default);
}
var middleware_loader_entry_default = WRAPPED_ENTRY;
export {
  Limiter,
  Lobby,
  __INTERNAL_WRANGLER_MIDDLEWARE__,
  middleware_loader_entry_default as default
};
//# sourceMappingURL=index.js.map
