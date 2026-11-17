// The pack builder page: reads the form, runs worker.js, offers the zip.
import { parseChannels, parseExtras } from "./pack.js";

const $ = (id) => document.getElementById(id);
const form = $("form");
const STORE = "seventv-pack-builder";
const PAGE_URL = location.origin + location.pathname;

function settingsFromForm() {
  return {
    channels: $("channels").value.trim(),
    global: $("global").checked,
    extras: $("extras").value,
    format: form.elements.format.value,
  };
}

function applySettings(s) {
  if (!s) return;
  if (typeof s.channels === "string") $("channels").value = s.channels;
  if (typeof s.global === "boolean") $("global").checked = s.global;
  if (typeof s.extras === "string" && s.extras.trim()) {
    $("extras").value = s.extras;
    $("extrasBox").open = true;
  }
  if (s.format === "tga" || s.format === "blp") form.elements.format.value = s.format;
}

// A link opens with its settings filled in: ?channels=a,b&global=0&format=tga&emotes=line|line
function settingsFromUrl() {
  const q = new URLSearchParams(location.search);
  if (![...q.keys()].length) return null;
  const s = {};
  if (q.has("channels")) s.channels = q.get("channels").split(",").join(", ");
  if (q.has("global")) s.global = q.get("global") !== "0";
  if (q.has("format")) s.format = q.get("format");
  if (q.has("emotes")) s.extras = q.get("emotes").split("|").join("\n");
  return s;
}

function shareUrl(s) {
  const q = new URLSearchParams();
  const { logins } = parseChannels(s.channels);
  if (logins.length) q.set("channels", logins.join(","));
  if (!s.global) q.set("global", "0");
  if (s.format === "tga") q.set("format", "tga");
  const extras = parseExtras(s.extras).map((e) => e.line);
  if (extras.length) q.set("emotes", extras.join("|"));
  const qs = q.toString();
  return PAGE_URL + (qs ? "?" + qs : "");
}

function remember(s) {
  try {
    localStorage.setItem(STORE, JSON.stringify(s));
  } catch {
    /* storage unavailable: fine */
  }
}

function recall() {
  try {
    return JSON.parse(localStorage.getItem(STORE) || "null");
  } catch {
    return null;
  }
}

applySettings(settingsFromUrl() || recall());

const bytes = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);

let worker = null;
let lastUrl = null;

function setStatus(text, count = "", fraction = null, slow = false) {
  $("status").hidden = false;
  $("statusText").textContent = text;
  $("statusCount").textContent = count;
  $("barFill").style.width = fraction === null ? "0%" : `${Math.round(fraction * 100)}%`;
  $("barFill").classList.toggle("busy", fraction === null);
  $("slowNote").hidden = !slow;
}

function showProblems(problems) {
  const box = $("problemsBox");
  const list = $("problems");
  list.replaceChildren();
  box.hidden = problems.length === 0;
  if (!problems.length) return;
  $("problemsSummary").textContent = `${problems.length} thing${problems.length === 1 ? "" : "s"} couldn't be added`;
  const retry = problems.filter((p) => p.retry).length;
  $("retryNote").hidden = retry === 0;
  $("retryNote").textContent =
    `7TV's server didn't send ${retry === 1 ? "one emote" : `${retry} emotes`} (it's often overloaded). ` +
    "Click Build again to retry: emotes you already have come from your browser's cache, so it's much quicker.";
  for (const p of problems) {
    const li = document.createElement("li");
    const b = document.createElement("strong");
    b.textContent = p.what;
    li.append(b, `: ${p.why}`);
    list.append(li);
  }
  box.open = problems.length <= 5 || retry > 0;
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  const s = settingsFromForm();
  const { logins, bad } = parseChannels(s.channels);
  const extras = parseExtras(s.extras);
  if (!logins.length && !s.global && !extras.length) {
    setStatus("Add a Twitch channel, keep the global emotes, or add some emotes first.");
    return;
  }
  remember(s);
  if (worker) worker.terminate();
  try {
    worker = new Worker(new URL("worker.js", import.meta.url), { type: "module" });
  } catch {
    setStatus("This browser can't run the pack builder. Try a recent Chrome, Edge or Firefox.");
    return;
  }
  const early = bad.map((b) => ({ what: b, why: "that doesn't look like a Twitch channel name" }));
  $("build").disabled = true;
  $("result").hidden = true;
  showProblems(early);
  setStatus("Starting...");

  worker.onmessage = (e) => {
    const m = e.data;
    if (m.type === "progress") {
      if (m.phase === "lookup") setStatus(`Looking up ${m.text}...`);
      else if (m.phase === "download") setStatus("Downloading and converting emotes", `${m.done} / ${m.total}`, m.total ? m.done / m.total : 1, true);
      else if (m.phase === "zip") setStatus("Zipping...", "", 1);
    } else if (m.type === "done") {
      $("build").disabled = false;
      if (lastUrl) URL.revokeObjectURL(lastUrl);
      const problems = [...early, ...m.problems];
      if (m.emotes === 0) {
        setStatus("No emotes could be added.", "", 0);
        showProblems(problems);
        return;
      }
      lastUrl = URL.createObjectURL(new Blob([m.zip], { type: "application/zip" }));
      $("download").href = lastUrl;
      setStatus(`Your pack is ready: ${m.emotes} emote${m.emotes === 1 ? "" : "s"}.`, "", 1);
      const from = [m.channels.length ? m.channels.join(", ") : null, s.global ? "7TV globals" : null, ...(m.sets || [])].filter(Boolean);
      $("resultInfo").textContent = `${bytes(m.zip.length)}${from.length ? " - from " + from.join(", ") : ""}`;
      $("result").hidden = false;
      showProblems(problems);
      $("download").focus();
    } else if (m.type === "error") {
      $("build").disabled = false;
      setStatus(`Something went wrong: ${m.message}`);
    }
  };
  worker.onerror = (e) => {
    $("build").disabled = false;
    setStatus(`Something went wrong: ${e.message || "the builder stopped"}`);
  };
  worker.postMessage({ channels: logins, global: s.global, extras, blp: s.format !== "tga", url: PAGE_URL });
});

$("share").addEventListener("click", async () => {
  const url = shareUrl(settingsFromForm());
  try {
    await navigator.clipboard.writeText(url);
    $("share").textContent = "Link copied";
  } catch {
    window.prompt("Copy this link:", url);
  }
  setTimeout(() => ($("share").textContent = "Copy link to these settings"), 2000);
});

fetch("version.txt")
  .then((r) => (r.ok ? r.text() : ""))
  .then((v) => v && ($("version").textContent = `builder ${v.trim()}`))
  .catch(() => {});
