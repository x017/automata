// Lists what the engine's wasm module (the .so in the wheel) imports from
// outside Emscripten's `env` / `GOT.*` modules, and fails if there is any:
// Pyodide can only satisfy those (wasm-bindgen's placeholder module, for
// one, means a dependency pulled in browser glue meant for
// wasm32-unknown-unknown).
//
//   node web/ci/wasm_imports.mjs path/to/_core.so
import fs from "node:fs";

const module = new WebAssembly.Module(fs.readFileSync(process.argv[2]));
const imports = WebAssembly.Module.imports(module);
const foreign = imports.filter((i) => i.module !== "env" && !i.module.startsWith("GOT."));
console.log(`${imports.length} imports, ${foreign.length} from other modules`);
for (const i of foreign) console.log(`  ${i.module}.${i.name} (${i.kind})`);
process.exit(foreign.length ? 1 : 0);
