"""python-for-android's sdl3 recipe without SDL3_image, SDL3_mixer and SDL3_ttf.

The engine loads images, decodes audio and draws text itself, so those
three libraries (about 2 MB per processor type, compressed) would only take
up space. p4a's launcher loads them only when they're there
(``PythonUtil.addLibraryIfExists``), and ``libmain`` links SDL3 alone.

SDL3 itself is built exactly as by the core recipe, which is loaded from
p4a's own folder.
"""

import importlib.util
from os.path import dirname, exists, join

import pythonforandroid

_core_file = join(dirname(pythonforandroid.__file__), "recipes", "sdl3", "__init__.py")
_spec = importlib.util.spec_from_file_location("automata_p4a_core_sdl3", _core_file)
_core = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_core)


class LibSDL3Recipe(_core.LibSDL3Recipe):
    """SDL3 alone."""

    # The same ndk-build also makes libmain from p4a's start.c, which needs
    # Python's headers: CPython first. (In the core recipe that order came
    # about through the satellite libraries.)
    depends = ["python3"]

    def get_recipe_dir(self):
        return dirname(_core_file)

    def should_build(self, arch):
        libdir = join(self.get_build_dir(arch.arch), "../..", "libs", arch.arch)
        return not all(exists(join(libdir, x)) for x in ("libmain.so", "libSDL3.so"))


recipe = LibSDL3Recipe()
