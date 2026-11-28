// Rewrites Pyodide's python_stdlib.zip for web builds (phase 5 of the web
// export: download size and start-up time).
//
// - Leaves out the modules web/ci/stdlib-drop.txt names: ones a game in a
//   browser can't use (processes, terminals, sockets for mail) and
//   developer tools (debuggers, test runners, the REPL).
// - Compiles the modules a game imports at start (web/ci/stdlib-hot.txt,
//   plus everything they import) to bytecode: Python can't cache bytecode
//   for modules in a zip, so it would compile their source on every start.
//   The rest stays source, which is smaller.
//
// It runs in Pyodide itself (under Node), so the bytecode is for exactly the
// CPython the page runs.
//
//   node web/ci/stdlib.mjs PYODIDE_DIR IN.zip OUT.zip
//   node web/ci/stdlib.mjs PYODIDE_DIR IN.zip OUT.zip --all     (compile everything)
//   node web/ci/stdlib.mjs PYODIDE_DIR IN.zip OUT.zip --none    (only drop)
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const [dir, input, output, mode = "--hot"] = process.argv.slice(2);
if (!dir || !input || !output) {
  console.error("usage: node stdlib.mjs PYODIDE_DIR IN.zip OUT.zip [--all|--none]");
  process.exit(2);
}
const list = (file) =>
  fs
    .readFileSync(path.join(here, file), "utf8")
    .split("\n")
    .map((l) => l.replace(/#.*/, "").trim())
    .filter(Boolean);

const { loadPyodide } = await import(pathToFileURL(path.resolve(dir, "pyodide.mjs")).href);
const py = await loadPyodide({ indexURL: path.resolve(dir) + "/" });
py.FS.mkdirTree("/host");
py.FS.mount(py.FS.filesystems.NODEFS, { root: "/" }, "/host");
py.globals.set(
  "ARGS",
  py.toPy({
    input: "/host" + path.resolve(input),
    output: "/host" + path.resolve(output),
    drop: list("stdlib-drop.txt"),
    hot: list("stdlib-hot.txt"),
    mode,
  }),
);
const report = py.runPython(`
import fnmatch, importlib.util, marshal, sys, zipfile

def module_name(path):
    name = path[: -len(".py")].replace("/", ".")
    return name.removesuffix(".__init__")

def under(name, patterns):
    """Whether a pattern (a module, its package, or a glob) covers \`name\`;
    \`!pattern\` keeps what an earlier pattern dropped."""
    hit = False
    for p in patterns:
        keep = p.startswith("!")
        p = p.lstrip("!")
        if name == p or name.startswith(p + ".") or fnmatch.fnmatchcase(name, p):
            hit = not keep
    return hit

src = zipfile.ZipFile(ARGS["input"])
files = {i.filename: i for i in src.infolist() if not i.is_dir()}
# Modules, and data files inside dropped packages.
def package_of(f):
    return f.rsplit("/", 1)[0].replace("/", ".") if "/" in f else ""
dropped = {
    f for f in files
    if (under(module_name(f), ARGS["drop"]) if f.endswith(".py") else bool(package_of(f)) and under(package_of(f), ARGS["drop"]))
}

# What the hot modules import, found by importing them here (this Pyodide
# has the original standard library): everything they pull in is hot too.
# A dropped module they import is a mistake in the lists.
hot = set()
if ARGS["mode"] == "--hot":
    before = set(sys.modules)
    for name in ARGS["hot"]:
        try:
            __import__(name)
        except ImportError as e:
            print(f"(hot module {name} doesn't import here: {e})")
    hot = set(sys.modules)
    stdlib = {module_name(f) for f in files if f.endswith(".py")}
    hot &= stdlib
    clash = sorted(m for m in hot if under(m, ARGS["drop"]))
    if clash:
        raise SystemExit(f"stdlib-drop.txt drops modules a game imports at start: {clash}")

compiled = kept = 0
size_in = sum(i.compress_size for i in files.values())
with zipfile.ZipFile(ARGS["output"], "w", zipfile.ZIP_DEFLATED, compresslevel=9) as dst:
    for name in sorted(files):
        if name in dropped:
            continue
        data = src.read(name)
        out = name
        if name.endswith(".py") and (ARGS["mode"] == "--all" or module_name(name) in hot):
            code = compile(data, name, "exec", dont_inherit=True)
            # A sourceless .pyc: zipimport loads it without checking a stamp.
            data = importlib.util.MAGIC_NUMBER + bytes(12) + marshal.dumps(code)
            out = name[:-3] + ".pyc"
            compiled += 1
        info = zipfile.ZipInfo(out, (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        dst.writestr(info, data, compresslevel=9)
        kept += 1
dst_size = __import__("os").path.getsize(ARGS["output"])
f"python_stdlib.zip: {len(files)} files, {size_in} bytes -> {kept} files ({len(dropped)} dropped, {compiled} compiled), {dst_size} bytes"
`);
console.log(report);
