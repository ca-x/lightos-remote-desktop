import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

execFileSync("protoc", [
  "--plugin=protoc-gen-es=./node_modules/.bin/protoc-gen-es",
  "--es_out=web/src/gen", "--es_opt=target=js+dts", "-I", "proto", "proto/wire.proto",
], { stdio: "inherit" });
for (const name of ["wire_pb.js", "wire_pb.d.ts"]) {
  const path = `web/src/gen/${name}`;
  writeFileSync(path, readFileSync(path, "utf8").trimEnd() + "\n");
}
