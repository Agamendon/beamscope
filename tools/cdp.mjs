// Evaluate JS in the BeamScope web view over the Chrome DevTools Protocol and print console output.
// Launch the app with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223 first.
// usage: node tools/cdp.mjs "<expression>" [listenMs]
const [expr = "1", listenMs = "1500"] = process.argv.slice(2);
const targets = await (await fetch("http://localhost:9223/json")).json();
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const send = (method, params = {}) =>
  new Promise((res) => {
    const i = ++id;
    pending.set(i, res);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
  } else if (m.method === "Runtime.consoleAPICalled") {
    console.log(`[console.${m.params.type}]`, m.params.args.map((a) => a.value ?? a.description).join(" "));
  } else if (m.method === "Runtime.exceptionThrown") {
    console.log("[exception]", m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
  }
};
await new Promise((r) => (ws.onopen = r));
await send("Runtime.enable");
const r = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true, timeout: 8000 });
console.log("result:", JSON.stringify(r.result?.result?.value ?? r.result?.exceptionDetails ?? r.result).slice(0, 2000));
await new Promise((r) => setTimeout(r, +listenMs));
ws.close();
