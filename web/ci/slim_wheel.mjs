// Readies the engine wheel for web builds (phase 5 of the web export:
// download size and start-up time):
//
// - compiles its Python modules to bytecode, as unchecked hash-based .pyc
//   files in __pycache__ (PEP 552): Python uses them without looking at
//   the source, which stays for tracebacks. Without them every start
//   compiles automata's Python again (script_api.py alone takes ~0.25 s);
// - leaves out what only editors read: type stubs (.pyi) and py.typed;
// - rewrites RECORD to match.
//
// It runs in Pyodide itself (under Node), so the bytecode is for exactly the
// CPython the page runs. The same input gives the same wheel, byte for byte.
//
//   node web/ci/slim_wheel.mjs PYODIDE_DIR IN.whl OUT.whl
import path from "node:path";
import { pathToFileURL } from "node:url";

const [dir, input, output] = process.argv.slice(2);
if (!dir || !input || !output) {
  console.error("usage: node slim_wheel.mjs PYODIDE_DIR IN.whl OUT.whl");
  process.exit(2);
}
const { loadPyodide } = await import(pathToFileURL(path.resolve(dir, "pyodide.mjs")).href);
const py = await loadPyodide({ indexURL: path.resolve(dir) + "/" });
py.FS.mkdirTree("/host");
py.FS.mount(py.FS.filesystems.NODEFS, { root: "/" }, "/host");
py.globals.set("IN", "/host" + path.resolve(input));
py.globals.set("OUT", "/host" + path.resolve(output));
const report = py.runPython(`
import base64, hashlib, importlib.util, marshal, os, sys, zipfile

tag = sys.implementation.cache_tag
src = zipfile.ZipFile(IN)
names = [i.filename for i in src.infolist() if not i.is_dir()]
record = next(n for n in names if n.endswith(".dist-info/RECORD"))
files = {}
dropped = []
for name in names:
    if name == record:
        continue
    if name.endswith((".pyi", "/py.typed")) or "/__pycache__/" in name:
        dropped.append(name)
        continue
    data = src.read(name)
    files[name] = data
    if name.endswith(".py") and ".dist-info/" not in name:
        folder, file = name.rsplit("/", 1) if "/" in name else ("", name)
        code = compile(data, name, "exec", dont_inherit=True)
        # flags 0b01: hash-based, not checked against the source.
        pyc = importlib.util.MAGIC_NUMBER + (1).to_bytes(4, "little") + importlib.util.source_hash(data) + marshal.dumps(code)
        cached = f"{folder}/__pycache__/{file[:-3]}.{tag}.pyc".lstrip("/")
        files[cached] = pyc

def digest(data):
    return "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()

lines = [f"{n},{digest(d)},{len(d)}" for n, d in sorted(files.items())] + [f"{record},,"]
files[record] = ("\\n".join(lines) + "\\n").encode()
with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as dst:
    for name in sorted(files, key=lambda n: (n == record, n)):
        info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = 0o644 << 16
        dst.writestr(info, files[name], compresslevel=9)
pycs = sum(n.endswith(".pyc") for n in files)
f"{os.path.basename(IN)}: {os.path.getsize(IN)} -> {os.path.getsize(OUT)} bytes ({pycs} modules compiled, dropped {dropped})"
`);
console.log(report);
