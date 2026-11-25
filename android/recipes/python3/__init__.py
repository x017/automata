"""python-for-android's python3 recipe with a smaller standard library.

Everything about building CPython stays the core recipe's (it's loaded from
p4a's own folder, and its patches are applied from there). Only what goes
into ``stdlib.zip`` changes: on top of what p4a already leaves out (tests,
tkinter, idlelib, ensurepip, ...), the parts of the standard library a game
on a phone can't use or has no use for:

- the interactive tools: the REPL (``_pyrepl``), the debugger, ``pydoc``,
  ``doctest``, ``unittest`` and the profilers;
- desktop and server pieces: ``wsgiref``, ``xmlrpc``, ``dbm``, ``mailbox``;
- pure-Python fallbacks for modules that are built in C (``_pyio``,
  ``_pydatetime``, ``_pydecimal``);
- CPython's own test extension modules, and the CJK codecs (``shift_jis``,
  ``gbk``, ``big5``, ... for bytes in those encodings: Python text is
  Unicode, and UTF-8 and the other codecs stay);
- the SBOM files maturin puts in the engine's ``dist-info``.

Games keep ``json``, ``random``, ``math``, ``re``, ``dataclasses``,
``asyncio``, ``urllib``/``http`` (with ``ssl``), ``sqlite3``, ``zipfile``,
``csv``, ``xml``, ``logging`` and the rest. ``android/ci/apk_size.py``
shows what's left (the CI's "android: template size" check).
"""

import glob
import importlib.util
import os
from os.path import basename, dirname, join

import pythonforandroid

_core_file = join(dirname(pythonforandroid.__file__), "recipes", "python3", "__init__.py")
_spec = importlib.util.spec_from_file_location("automata_p4a_core_python3", _core_file)
_core = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_core)

# Folders left out wherever they are (p4a matches folder names).
EXTRA_DIRS = {
    "_pyrepl",
    "unittest",
    "wsgiref",
    "xmlrpc",
    "dbm",
    "profiling",
    "__phello__",
}

# Files left out (p4a matches file names; the stdlib is compiled to .pyc).
EXTRA_FILES = [
    "pydoc.pyc",
    "doctest.pyc",
    "pdb.pyc",
    "bdb.pyc",
    "profile.pyc",
    "cProfile.pyc",
    "pstats.pyc",
    "timeit.pyc",
    "trace.pyc",
    "tabnanny.pyc",
    "pyclbr.pyc",
    "mailbox.pyc",
    "imaplib.pyc",
    "poplib.pyc",
    "smtplib.pyc",
    "nntplib.pyc",
    "ftplib.pyc",
    "_pyio.pyc",
    "_pydecimal.pyc",
    "_pydatetime.pyc",
    "_osx_support.pyc",
    "_aix_support.pyc",
    "_ios_support.pyc",
]


# C extension modules left out of the bundle's ``modules/`` folder.
EXTRA_MODULES = [
    "_test*",
    "_ctypes_test*",
    "_xxtestfuzz*",
    "xxlimited*",
    "xxsubtype*",
    "_codecs_cn*",
    "_codecs_hk*",
    "_codecs_iso2022*",
    "_codecs_jp*",
    "_codecs_kr*",
    "_codecs_tw*",
    "_multibytecodec*",
]


class Python3Recipe(_core.Python3Recipe):
    """The core recipe, with a smaller ``stdlib.zip``."""

    stdlib_dir_blacklist = set(_core.Python3Recipe.stdlib_dir_blacklist) | EXTRA_DIRS
    stdlib_filen_blacklist = list(_core.Python3Recipe.stdlib_filen_blacklist) + EXTRA_FILES
    site_packages_dir_blacklist = set(_core.Python3Recipe.site_packages_dir_blacklist) | {"sboms"}

    def create_python_bundle(self, dirn, arch):
        site_packages = super().create_python_bundle(dirn, arch)
        removed = []
        for pattern in EXTRA_MODULES:
            for path in glob.glob(join(dirn, "modules", pattern)):
                os.remove(path)
                removed.append(basename(path))
        _core.info(f"automata: left out {len(removed)} C modules: {' '.join(sorted(removed))}")
        return site_packages

    def get_recipe_dir(self):
        # The patches and anything else the core recipe reads live in p4a.
        return dirname(_core_file)


recipe = Python3Recipe()
