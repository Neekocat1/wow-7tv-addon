// End-to-end test of the pack builder against a fake 7TV (same fixtures as helper-rs/tests).
// Run: node web/test/pack.test.mjs [path to pack.wasm] [where to write the test zip]
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { loadWasm, buildPack, parseChannels, parseExtras, channelLogin } from "../site/pack.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const wasmPath = process.argv[2] || path.join(root, "web/site/pack.wasm");
const zipOut = process.argv[3];
const fx = (f) => fs.readFileSync(path.join(root, "helper-rs/tests/fixtures", f));

const ANIM = "01ANIMATEDXXXXXXXXXXXXXXXX";
const STAT = "01STATICXXXXXXXXXXXXXXXXXX";
const WIDE = "60ae958e229664e8667aea38";
const CHAN = "01CHANNELEMOTEXXXXXXXXXXXX";
const ZW = "01ZEROWIDTHXXXXXXXXXXXXXXX";
const BROKEN = "01BROKENXXXXXXXXXXXXXXXXXX";
const host = (id, f, w, h, fmt) => ({ url: `//cdn.test/emote/${id}`, files: [{ name: f, width: w, height: h, format: fmt }] });
const H = {
  [ANIM]: host(ANIM, "2x.webp", 64, 64, "WEBP"), [STAT]: host(STAT, "2x.webp", 64, 64, "WEBP"),
  [WIDE]: host(WIDE, "2x.png", 128, 32, "PNG"), [CHAN]: host(CHAN, "2x.webp", 64, 64, "WEBP"),
  [ZW]: host(ZW, "2x.webp", 64, 64, "WEBP"), [BROKEN]: host(BROKEN, "2x.webp", 64, 64, "WEBP"),
};
const active = (id, name, animated, flags = 0) => ({ id, name, flags: 0, data: { id, name, animated, flags, host: H[id] } });
const files = {
  [`https://cdn.test/emote/${ANIM}/2x.webp`]: fx("anim.webp"),
  [`https://cdn.test/emote/${STAT}/2x.webp`]: fx("static.webp"),
  [`https://cdn.test/emote/${WIDE}/2x.png`]: fx("wide.png"),
  [`https://cdn.test/emote/${CHAN}/2x.webp`]: fx("static.webp"),
  [`https://cdn.test/emote/${BROKEN}/2x.webp`]: Buffer.from("not an image"),
};
const json = {
  "https://7tv.io/v3/emote-sets/global": { name: "Global", emotes: [active(ANIM, "catJAM", true), active(STAT, "OMEGALUL", false), active(ZW, "RainTime", false, 256)] },
  "https://api.ivr.fi/v2/twitch/user?login=tester": [{ id: "123", login: "tester" }],
  "https://api.ivr.fi/v2/twitch/user?login=nobody": [],
  "https://api.ivr.fi/v2/twitch/user?login=notseventv": [{ id: "999" }],
  "https://7tv.io/v3/users/twitch/123": { emote_set: { id: "SET1", name: "tester's set", emotes: [active(CHAN, "testerHi", false), active(STAT, "Clap", false), active(BROKEN, "Broken", false)] } },
  [`https://7tv.io/v3/emotes/${WIDE}`]: { id: WIDE, name: "WideDank", animated: false, host: H[WIDE] },
  "https://7tv.io/v3/emote-sets/01HKQT8EWR000ESSWF3625XCS4": { name: "Cool Set", emotes: [active(WIDE, "SetWide", false)] },
};
const calls = [];
async function fakeFetch(url, init) {
  calls.push(url);
  if (url === "https://7tv.io/v3/gql") {
    const q = JSON.parse(init.body).variables.query;
    const items = q === "peepoHappy" ? [{ id: WIDE, name: "peepoHappy", animated: false, host: H[WIDE] }] : [];
    return new Response(JSON.stringify({ data: { emotes: { items } } }));
  }
  if (files[url]) return new Response(files[url]);
  if (json[url]) return new Response(JSON.stringify(json[url]));
  return new Response("not found", { status: 404 });
}

// input parsing ----------------------------------------------------------------------------------
assert.equal(channelLogin("https://www.twitch.tv/GrimmyBear?x=1"), "grimmybear");
assert.equal(channelLogin("@xQc"), "xqc");
assert.equal(channelLogin("not a name!"), null);
assert.deepEqual(parseChannels("Tester, tester  nobody;bad!name"), { logins: ["tester", "nobody"], bad: ["bad!name"] });
assert.deepEqual(parseExtras("# comment\n\npeepoHappy happy\n https://7tv.app/emotes/x \n").map((e) => [e.ref, e.alias]),
  [["peepoHappy", "happy"], ["https://7tv.app/emotes/x", null]]);

// full build -----------------------------------------------------------------------------------
const wasm = await loadWasm(fs.readFileSync(wasmPath));
const events = [];
const res = await buildPack(
  {
    channels: ["tester", "nobody", "notseventv"],
    global: true,
    extras: parseExtras(`https://7tv.app/emotes/${WIDE}\npeepoHappy happy\ncatJAM dance\nNotARealEmote\nhttps://7tv.app/emote-sets/01HKQT8EWR000ESSWF3625XCS4\nOMEGALUL bad|name`),
    blp: true,
    url: "https://neekocat1.github.io/wow-7tv-addon/",
  },
  { wasm, fetch: fakeFetch, now: () => 1800000000, progress: (e) => events.push(e), concurrency: 3 },
);

const why = Object.fromEntries(res.problems.map((p) => [p.what, p.why]));
assert.match(why["nobody's channel"], /no Twitch channel/);
assert.match(why["notseventv's channel"], /doesn't use 7TV/);
assert.match(why["NotARealEmote"], /couldn't find a 7TV emote called NotARealEmote/);
assert.match(why["OMEGALUL bad|name"], /can't be used as an emote name/);
assert.ok(why["Broken"], "broken image reported");
assert.equal(res.problems.length, 5, JSON.stringify(res.problems));
assert.deepEqual(res.channels, ["tester"]);
assert.deepEqual(res.sets, ["Cool Set"]);
// catJAM, OMEGALUL, testerHi, Clap, WideDank, happy, dance, SetWide (RainTime zero-width, Broken failed)
assert.equal(res.emotes, 8);
assert.equal(res.textures, 4, "one texture per 7TV id");
assert.ok(events.some((e) => e.phase === "download" && e.done === e.total && e.total === 5));
assert.ok(!calls.some((u) => u.includes(ZW)), "zero-width emote never downloaded");

// unpack with Python's zipfile to make sure other tools read it
const tmp = fs.mkdtempSync("/tmp/packtest-");
const zipPath = zipOut || path.join(tmp, "pack.zip");
fs.writeFileSync(zipPath, res.zip);
console.log(`pack: ${res.emotes} emotes, ${res.textures} textures, ${res.zip.length} bytes -> ${zipPath}`);
console.log("problems:", res.problems.map((p) => `${p.what}: ${p.why}`).join(" | "));
process.stdout.write("ok\n");
