// Runs the pack build off the page's main thread, so the page stays responsive while hundreds of
// emotes are converted.
import { loadWasm, buildPack } from "./pack.js";

let wasmPromise = null;

self.onmessage = async (event) => {
  const opts = event.data;
  try {
    wasmPromise ??= loadWasm(fetch(new URL("pack.wasm", import.meta.url)));
    const wasm = await wasmPromise;
    const result = await buildPack(opts, {
      wasm,
      fetch: (...args) => fetch(...args),
      now: () => Math.floor(Date.now() / 1000),
      progress: (p) => self.postMessage({ type: "progress", ...p }),
    });
    self.postMessage({ type: "done", ...result }, [result.zip.buffer]);
  } catch (err) {
    wasmPromise = null; // a failed load can be retried
    self.postMessage({ type: "error", message: String((err && err.message) || err) });
  }
};
