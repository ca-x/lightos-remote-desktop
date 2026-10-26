import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const entryFiles = new Set(["index.html", "app.js", "style.css", "audio-worker.js", "audio-worklet.js"]);
for (const name of readdirSync("web/dist")) {
  if (!entryFiles.has(name)) throw new Error(`Unmapped frontend resource: ${name}`);
}

for (const name of ["app.js", "style.css", "audio-worker.js", "audio-worklet.js"]) {
  writeFileSync(`web/dist/${name}.gz`, gzipSync(readFileSync(`web/dist/${name}`), { level: 9 }));
}

const npmCli = process.env.npm_execpath;
if (!npmCli) throw new Error("Generate frontend assets through npm run build");
const roots = execFileSync(process.execPath, [npmCli, "ls", "--omit=dev", "--all", "--parseable"], { encoding: "utf8" }).trim().split(/\r?\n/).slice(1);
const notices = ["Frontend third-party components. Generated from the locked production dependency tree.\n"];
for (const root of [...new Set(roots)].sort()) {
  const info = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  notices.push(`\n=== ${info.name}@${info.version} (${info.license ?? "see package notices"}) ===\n`);
  const noticeRoot = readdirSync(root).some((name) => /^(licen[cs]e|copying|notice)(\.|$)/i.test(name))
    ? root : join("docs/licenses/frontend", `${info.name}-${info.version}`);
  if (!existsSync(noticeRoot)) throw new Error(`No license notice found for ${info.name}`);
  const files = readdirSync(noticeRoot).filter((name) => /^(licen[cs]e|copying|notice)(\.|$)/i.test(name));
  if (!files.length) throw new Error(`No license notice found for ${info.name}`);
  for (const file of files) notices.push(readFileSync(join(noticeRoot, file), "utf8"));
}
writeFileSync("web/dist/NOTICE.frontend.txt", notices.join("\n"));
