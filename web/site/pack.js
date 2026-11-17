// Builds a SevenTVEmotes_Local emote pack: fetches emote lists and images from 7TV, converts them
// with the WebAssembly module (the helper's own Rust code) and zips the folder.
// Runs in a Web Worker on the page, and in Node for tests (fetch and the clock are passed in).

import { makeZip } from "./zip.js";

const enc = new TextEncoder();
const dec = new TextDecoder();

// Operation numbers of the WebAssembly module's `call` (see web/wasm/src/lib.rs).
const OP = {
  version: 0, url: 1, setEmotes: 2, setName: 3, emote: 4, twitchId: 5, channelSet: 6,
  parseRef: 7, searchBody: 8, searchPick: 9, resolve: 10, lua: 11, toc: 12,
  texturePath: 13, sourceLabel: 14, validName: 15,
};

export class Wasm {
  constructor(exports) {
    this.x = exports;
  }
  #put(bytes) {
    const p = this.x.alloc(bytes.length);
    new Uint8Array(this.x.memory.buffer, p, bytes.length).set(bytes);
    return p;
  }
  #out() {
    return new Uint8Array(this.x.memory.buffer, this.x.out_ptr(), this.x.out_len()).slice();
  }
  call(op, obj = {}) {
    const bytes = enc.encode(JSON.stringify(obj));
    const p = this.#put(bytes);
    const rc = this.x.call(op, p, bytes.length);
    this.x.dealloc(p, bytes.length);
    const text = dec.decode(this.#out());
    if (rc !== 0) throw new Error(text);
    return JSON.parse(text);
  }
  // Emote file bytes -> { layout, data } (texture file contents).
  convert(bytes, blp) {
    const p = this.#put(bytes);
    const rc = this.x.convert(p, bytes.length, blp ? 1 : 0);
    this.x.dealloc(p, bytes.length);
    const out = this.#out();
    if (rc !== 0) throw new Error(dec.decode(out));
    const v = new DataView(out.buffer);
    const n = (i) => v.getUint32(i * 4, true);
    return {
      layout: { w: n(0), h: n(1), tw: n(2), th: n(3), frames: n(4), cols: n(5), ms: n(6) },
      data: out.subarray(28),
    };
  }
}

export async function loadWasm(source) {
  // source: a Response (or promise of one) or the module's bytes.
  const res = await source;
  let instance;
  if (typeof Response !== "undefined" && res instanceof Response) {
    try {
      ({ instance } = await WebAssembly.instantiateStreaming(res.clone(), {}));
    } catch {
      ({ instance } = await WebAssembly.instantiate(await res.arrayBuffer(), {}));
    }
  } else {
    ({ instance } = await WebAssembly.instantiate(res, {}));
  }
  return new Wasm(instance.exports);
}

class NotFound extends Error {}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// A response's whole body, calling `alive()` whenever more of it arrives.
async function readBody(res, alive) {
  if (!res.body) return new Uint8Array(await res.arrayBuffer());
  const reader = res.body.getReader();
  const parts = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    alive();
    parts.push(value);
    size += value.length;
  }
  const out = new Uint8Array(size);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

// A Twitch login from "name", "@name" or a twitch.tv link; null if it can't be one.
export function channelLogin(text) {
  let s = String(text || "").trim();
  const m = s.match(/twitch\.tv\/([^/?#\s]+)/i);
  if (m) s = m[1];
  s = s.replace(/^@/, "").toLowerCase();
  return /^[a-z0-9_]{1,25}$/.test(s) ? s : null;
}

// "a, b c" -> ["a", "b", "c"] (logins, de-duplicated); bad entries come back in `bad`.
export function parseChannels(text) {
  const logins = [];
  const bad = [];
  for (const part of String(text || "").split(/[\s,;]+/)) {
    if (!part) continue;
    const login = channelLogin(part);
    if (!login) bad.push(part);
    else if (!logins.includes(login)) logins.push(login);
  }
  return { logins, bad };
}

// Extra emotes, one per line: "<7tv.app link, emote id, set link or name> [new name]".
export function parseExtras(text) {
  const out = [];
  for (const raw of String(text || "").split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const [ref, alias] = line.split(/\s+/);
    out.push({ ref, alias: alias || null, line });
  }
  return out;
}

export async function buildPack(opts, env) {
  const { wasm, now } = env;
  const fetchFn = env.fetch;
  const progress = env.progress || (() => {});
  // 7TV's image server is often slow (their developers blame DDoS attacks): a file can take 10-30 s
  // or never arrive. Many downloads at once keep the total time down, and a download that goes
  // quiet for `idleMs` is dropped and retried so it can't hold up a slot forever. Slow but steady
  // downloads are left alone, since on a slow connection a big animated emote can take a while.
  const concurrency = env.concurrency || 16;
  const idleMs = env.idleMs || 25000;
  const blp = opts.blp !== false;
  const problems = []; // { what, why }

  async function request(url, init, kind, tries = 3) {
    const host = new URL(url).host;
    const quiet = `${host} stopped answering (nothing for ${Math.round(idleMs / 1000)} s)`;
    let last;
    for (let attempt = 0; attempt < tries; attempt++) {
      const abort = new AbortController();
      let timer = setTimeout(() => abort.abort(), idleMs);
      const alive = () => {
        clearTimeout(timer);
        timer = setTimeout(() => abort.abort(), idleMs);
      };
      try {
        let res;
        try {
          res = await fetchFn(url, { ...init, signal: abort.signal });
        } catch (e) {
          last = abort.signal.aborted ? new Error(quiet) : new Error(`couldn't reach ${host} (${e.message})`);
          await sleep(800 * (attempt + 1));
          continue;
        }
        if (res.status === 404 || res.status === 400) throw new NotFound(url);
        if (res.ok) {
          let body;
          try {
            body = await readBody(res, alive);
          } catch (e) {
            last = abort.signal.aborted ? new Error(quiet) : new Error(`the download from ${host} broke off (${e.message})`);
            await sleep(800 * (attempt + 1));
            continue;
          }
          if (kind === "bytes") return body;
          try {
            return JSON.parse(dec.decode(body));
          } catch {
            throw new Error(`${host} sent something unreadable`);
          }
        }
        last = new Error(`${host} answered ${res.status}`);
        if (res.status !== 429 && res.status < 500) break;
        await sleep(1000 * (attempt + 1));
      } finally {
        clearTimeout(timer);
      }
    }
    throw last;
  }
  const getJson = (url) => request(url, undefined, "json");
  const getBytes = (url, tries) => request(url, undefined, "bytes", tries);
  const url = (kind, arg = "") => wasm.call(OP.url, { kind, arg });

  const sources = []; // [key, emotes] in priority order
  const labels = {};
  const channels = [];
  const sets = [];

  // 1. Emote lists ----------------------------------------------------------------------------
  if (opts.global) {
    progress({ phase: "lookup", text: "7TV global emotes" });
    try {
      sources.push(["global", wasm.call(OP.setEmotes, { json: await getJson(url("global")) })]);
    } catch (e) {
      problems.push({ what: "7TV global emotes", why: e.message });
    }
  }

  // Channels in the order given, highest priority first: if two have an emote with the same name,
  // the one listed first wins. (Sources go lowest priority first, so they're added reversed.)
  const channelSources = [];
  for (const login of opts.channels || []) {
    progress({ phase: "lookup", text: `${login}'s channel` });
    try {
      let twitchId;
      try {
        twitchId = wasm.call(OP.twitchId, { json: await getJson(url("twitch_login", login)), login });
      } catch (e) {
        if (e instanceof NotFound || /Couldn't find/.test(e.message)) throw new Error("no Twitch channel with that name");
        throw e;
      }
      let user;
      try {
        user = await getJson(url("twitch_user", twitchId));
      } catch (e) {
        if (e instanceof NotFound) throw new Error("this channel doesn't use 7TV");
        throw e;
      }
      let { set, fetch: setId } = wasm.call(OP.channelSet, { json: user });
      if (setId) set = await getJson(url("set", setId));
      const list = wasm.call(OP.setEmotes, { json: set });
      if (list.length === 0) throw new Error("this channel has no 7TV emotes");
      const key = `channel:${login}`;
      channelSources.push([key, list]);
      labels[key] = wasm.call(OP.sourceLabel, { key });
      channels.push(login);
    } catch (e) {
      problems.push({ what: `${login}'s channel`, why: e.message });
    }
  }

  sources.push(...channelSources.reverse());

  const manual = [];
  for (const extra of opts.extras || []) {
    progress({ phase: "lookup", text: extra.ref });
    try {
      if (extra.alias && !wasm.call(OP.validName, { name: extra.alias })) {
        throw new Error(`"${extra.alias}" can't be used as an emote name in WoW chat (no spaces or |)`);
      }
      const ref = wasm.call(OP.parseRef, { ref: extra.ref });
      if (ref.kind === "set") {
        let json;
        try {
          json = await getJson(url("set", ref.value));
        } catch (e) {
          throw e instanceof NotFound ? new Error("couldn't find that emote set on 7TV") : e;
        }
        const name = wasm.call(OP.setName, { json, id: ref.value });
        const key = `set:${ref.value}`;
        sources.push([key, wasm.call(OP.setEmotes, { json })]);
        labels[key] = wasm.call(OP.sourceLabel, { key, set_label: name });
        sets.push(name);
        continue;
      }
      let em;
      if (ref.kind === "id") {
        try {
          em = wasm.call(OP.emote, { json: await getJson(url("emote", ref.value)) });
        } catch (e) {
          throw e instanceof NotFound ? new Error("couldn't find that emote on 7TV; check the link") : e;
        }
      } else {
        // the emote that name means in the pack so far (highest priority source first)
        for (const [, list] of [...sources].reverse()) em = em || list.find((x) => x.name === ref.value);
        if (!em) {
          let found;
          try {
            found = await request(
              url("gql"),
              {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(wasm.call(OP.searchBody, { name: ref.value })),
              },
              "json",
            );
          } catch (e) {
            throw e instanceof NotFound ? new Error("7TV search didn't accept that name") : e;
          }
          em = wasm.call(OP.searchPick, { json: found, name: ref.value });
        }
        if (!em) throw new Error(`couldn't find a 7TV emote called ${ref.value}; use its 7tv.app link instead`);
      }
      const name = extra.alias || em.name;
      if (!wasm.call(OP.validName, { name })) throw new Error(`"${name}" can't be used as an emote name in WoW chat`);
      manual.push({ ...em, name });
    } catch (e) {
      problems.push({ what: extra.line || extra.ref, why: e.message });
    }
  }
  if (manual.length) sources.push(["manual", manual]);

  // 2. Pick emotes and build textures ----------------------------------------------------------
  const chosen = wasm.call(OP.resolve, { sources });
  const namesById = new Map();
  for (const c of chosen) {
    if (!namesById.has(c.id)) namesById.set(c.id, { url: c.url, names: [] });
    namesById.get(c.id).names.push(c.name);
  }
  const tasks = [...namesById.entries()];
  const built = {};
  const files = [];
  let done = 0;
  progress({ phase: "download", done, total: tasks.length });

  let next = 0;
  async function worker() {
    while (next < tasks.length) {
      const [id, { url: fileUrl, names }] = tasks[next++];
      try {
        if (!fileUrl) throw new Error("7TV has no usable image for it");
        let bytes;
        try {
          bytes = await getBytes(fileUrl);
        } catch (e) {
          if (e instanceof NotFound) throw e;
          // 7TV's server tends to break off big files (large animated emotes) when it's busy. The
          // half-size version is about a third of the size and is scaled up to the same size in
          // chat, so a slightly softer emote beats a missing one.
          const small = fileUrl.replace(/\/2x\.(\w+)$/, "/1x.$1");
          if (small !== fileUrl) {
            try {
              bytes = await getBytes(small, 2);
            } catch {
              // report the original problem
            }
          }
          if (!bytes) {
            e.retry = true; // worth another try later
            throw e;
          }
        }
        const { layout, data } = wasm.convert(bytes, blp);
        built[id] = layout;
        files.push({ name: `SevenTVEmotes_Local/${wasm.call(OP.texturePath, { id, blp })}`, data });
      } catch (e) {
        problems.push({ what: names.join(", "), why: e.message, retry: !!e.retry });
      }
      done++;
      progress({ phase: "download", done, total: tasks.length });
    }
  }
  await Promise.all(Array.from({ length: Math.min(concurrency, tasks.length) }, worker));

  // 3. Local.lua, .toc, zip -----------------------------------------------------------------------
  progress({ phase: "zip" });
  const lua = wasm.call(OP.lua, {
    info: { built: now(), url: opts.url || "", blp, global: !!opts.global, channels, sets },
    sources,
    built,
    labels,
  });
  files.sort((a, b) => (a.name < b.name ? -1 : 1));
  const entries = [
    { name: "SevenTVEmotes_Local/SevenTVEmotes_Local.toc", data: enc.encode(wasm.call(OP.toc)) },
    { name: "SevenTVEmotes_Local/Local.lua", data: enc.encode(lua) },
    ...files,
  ];
  const zip = await makeZip(entries, { compress: opts.compressZip !== false });
  const emotes = chosen.filter((c) => built[c.id]).length;
  return { zip, emotes, textures: files.length, problems, channels, sets, version: wasm.call(OP.version) };
}
