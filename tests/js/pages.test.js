// Tests for the logic inside the page scripts. The pages are mostly DOM wiring, but a few rules decide
// what a person gets: which door a request goes through, which words appear instead of internals, and
// that every string on screen has a German and a Slovenian translation. Run: node tests/js/pages.test.js
const fs = require("fs");
const path = require("path");

const UI = path.join(__dirname, "..", "..", "src", "genesis-agentd", "ui");
let failures = 0;
function check(name, fn) {
  try { fn(); console.log("ok   " + name); }
  catch (e) { failures++; console.log("FAIL " + name + "\n     " + e.message); }
}
function assert(cond, msg) { if (!cond) throw new Error(msg); }

const palette = fs.readFileSync(path.join(UI, "palette.js"), "utf8");
const workspace = fs.readFileSync(path.join(UI, "workspace.js"), "utf8");

// ---- the one door: a request to make something opens the maker, a question goes to chat ----------
const MAKE = eval(palette.match(/const MAKE=(\/.*\/i);/)[1]);
check("a request to build opens the maker", () => {
  for (const t of ["a pomodoro timer with a bell", "make me a checklist", "build a website for the club",
                   "erstelle eine Webseite für den Verein", "naredi aplikacijo za opravila",
                   "rename the photos in a folder by date", "a word-count tool for text files"]) {
    assert(MAKE.test(t), `should make: ${t}`);
  }
});
check("a question goes to chat", () => {
  for (const t of ["what is the capital of Slovenia", "why is my wifi slow", "translate this to German",
                   "kaj sem napisal o Lizboni", "was steht in dieser Datei", "summarise this document",
                   "is an update waiting?"]) {
    assert(!MAKE.test(t), `should be a question: ${t}`);
  }
});

// ---- no internals on screen ----------------------------------------------------------------------
check("the maker shows plain words, not role ids or raw states", () => {
  assert(/small model/.test(workspace), "the model badge says small model");
  assert(/waiting for you/.test(workspace), "a waiting session says so in words");
  assert(!/textContent=.?'model: '/.test(workspace), "no raw 'model: fast' badge");
});

// ---- every page's dictionary covers every string it shows ------------------------------------------
for (const page of ["workspace", "palette", "chat", "settings"]) {
  check(`${page}: German and Slovenian cover the same strings`, () => {
    const src = fs.readFileSync(path.join(UI, page + ".js"), "utf8");
    const m = src.match(/const I18N=(\{[\s\S]*?\});\s*const LANG/);
    if (!m) return; // a page without its own dictionary
    const d = eval("(" + m[1] + ")");
    const de = Object.keys(d.de || {}), sl = Object.keys(d.sl || {});
    const missing = de.filter((k) => !sl.includes(k)).concat(sl.filter((k) => !de.includes(k)));
    assert(missing.length === 0, `these have only one translation: ${missing.slice(0, 5).join(" | ")}`);
    assert(de.length > 5, "the dictionary is not empty");
  });
}

// ---- the token never sits in a page that was fetched without one ------------------------------------
check("pages take the token from a placeholder, never hard-code one", () => {
  for (const page of ["workspace", "palette", "chat", "settings"]) {
    const src = fs.readFileSync(path.join(UI, page + ".js"), "utf8");
    assert(/__GENESIS_TOKEN__/.test(src), `${page} expects the placeholder`);
    assert(!/[0-9a-f]{32,}/.test(src), `${page} has something that looks like a real token in it`);
  }
});

process.exit(failures === 0 ? 0 : 1);
