"""python-for-android recipe for the AUTOMATA engine (`import automata`).

The source is a staged copy of the repository (android/build_apk.sh sets
P4A_automata_DIR); maturin builds android/engine, which wraps the Python
bindings with an SDL3 window driver, into the `automata` package.
"""

from os.path import join

from pythonforandroid.recipe import RustCompiledComponentsRecipe


class AutomataRecipe(RustCompiledComponentsRecipe):
    version = "0.1.0"
    url = None  # always P4A_automata_DIR
    site_packages_name = "automata"
    # The driver links the launcher's libSDL3.so.
    depends = ["sdl3"]

    def get_recipe_env(self, arch, **kwargs):
        env = super().get_recipe_env(arch, **kwargs)
        bootstrap = self.ctx.bootstrap.build_dir
        for libs in (join(bootstrap, "libs", arch.arch), join(bootstrap, "obj", "local", arch.arch)):
            env["RUSTFLAGS"] += f" -Clink-arg=-L{libs}"
        # 16 KB memory pages (required for new Play Store apps).
        env["RUSTFLAGS"] += " -Clink-arg=-Wl,-z,max-page-size=16384"
        # No vcpkg or pkg-config probing for SDL: link the one above.
        env["VCPKGRS_DISABLE"] = "1"
        return env


recipe = AutomataRecipe()
