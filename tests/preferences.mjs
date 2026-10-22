import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";
import vm from "node:vm";
import { gunzipSync } from "node:zlib";

const source = stripTypeScriptTypes(readFileSync(new URL("../web/src/preferences.ts", import.meta.url), "utf8")).replace(/^export /gm, "");
function preferences(storage) {
  const context = vm.createContext({ localStorage: storage });
  vm.runInContext(source, context);
  return (expression) => vm.runInContext(expression, context);
}

test("missing, malformed and blocked storage preserve working defaults", () => {
  for (const storage of [
    { getItem: () => null },
    { getItem: () => "unexpected" },
    { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); } },
  ]) {
    const run = preferences(storage);
    assert.equal(run('readPreference(themeKey, themes, "system")'), "system");
    assert.equal(run('readPreference(languageKey, languages, "system")'), "system");
    assert.doesNotThrow(() => run('savePreference(themeKey, "light")'));
  }
});

test("explicit preferences round-trip without changing transport settings", () => {
  const saved = new Map();
  const run = preferences({ getItem: (key) => saved.get(key), setItem: (key, value) => saved.set(key, value) });
  run('savePreference(themeKey, "dark"); savePreference(languageKey, "zh")');
  assert.equal(run('readPreference(themeKey, themes, "system")'), "dark");
  assert.equal(run('readPreference(languageKey, languages, "system")'), "zh");
  assert.equal(saved.size, 2);
});

test("browser language fallback and explicit language selection", () => {
  const run = preferences({});
  assert.equal(run('resolveLanguage("system", ["fr-FR", "zh-TW"])'), "zh");
  assert.equal(run('resolveLanguage("system", ["en-US", "zh-CN"])'), "en");
  assert.equal(run('resolveLanguage("system", ["ja-JP"])'), "en");
  assert.equal(run('resolveLanguage("zh", ["en-US"])'), "zh");
  assert.equal(run('resolveLanguage("en", ["zh-CN"])'), "en");
});

test("system theme responds to OS state; explicit themes do not", () => {
  const run = preferences({});
  assert.equal(run('resolveTheme("system", true)'), "dark");
  assert.equal(run('resolveTheme("system", false)'), "light");
  assert.equal(run('resolveTheme("light", true)'), "light");
  assert.equal(run('resolveTheme("dark", false)'), "dark");
});

test("precompressed embedded assets exactly match the production frontend", () => {
  for (const name of ["app.js", "style.css"]) {
    const raw = readFileSync(new URL(`../web/dist/${name}`, import.meta.url));
    const compressed = readFileSync(new URL(`../web/dist/${name}.gz`, import.meta.url));
    assert.deepEqual(gunzipSync(compressed), raw);
    assert.ok(compressed.length < raw.length);
  }
  const notices = readFileSync(new URL("../web/dist/NOTICE.frontend.txt", import.meta.url), "utf8");
  for (const dependency of ["react@", "react-dom@", "@headlessui/react@", "lucide-react@", "i18next@", "react-i18next@"]) assert.ok(notices.includes(dependency));
});
