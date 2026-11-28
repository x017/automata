"""A pause menu and a settings screen, drawn with ``ctx.ui``.

The game starts paused, on the menu. Resume plays: the shapes turn, and a
click anywhere in the game gives them a kick. Clicks on the HUD's Menu
button belong to the UI, so they never kick. Settings changes the volume,
the UI size and the graphics through ``ctx.settings``, which keeps them
between runs, along with a "Spin meter" toggle stored there too.

How it's built:
- **Screens:** "menu" and "settings" are ``ctx.ui.screens``. Each is
  modal, so the HUD under it doesn't react. Escape (or the gamepad's B)
  closes the top one, and P, Escape or Start opens the menu.
- **Layout:** columns and rows anchored to the screen. ``design_size``
  lays everything out for 1280x720 and scales it to fit any window.
- **Keys and gamepads:** arrow keys or the d-pad move between buttons,
  Enter or A presses them, and left/right move a focused slider.
- **Font:** titles and the HUD's numbers use a pixel font from the
  project's ``fonts/`` folder.
"""

#: Press Start 2P (SIL Open Font License, see fonts/PressStart2P-OFL.txt).
PIXEL = "fonts/PressStart2P-Regular.ttf"
PRESETS = ("low", "medium", "high", "ultra")
UI_SIZES = (("Small", 0.8), ("Normal", 1.0), ("Large", 1.25))


def start(entity, ctx):
    entity.state["kicks"] = 0
    ctx.ui.design_size = (1280, 720)
    ctx.settings.bind("pause", "p", "escape", "pad_start")
    ctx.ui.screens.show("menu", pause=True)  # the game starts on the menu


def update(entity, ctx):
    ui = ctx.ui
    screens = ui.screens
    # Escape on an open screen is the UI's (it closes it), so this only
    # sees it while playing.
    if ctx.input.just_pressed("pause"):
        if screens.top:
            screens.clear()
        else:
            screens.show("menu", pause=True)
    if not screens.paused and ctx.input.mouse_just_pressed("left"):
        # ctx.input never sees presses that began on the UI.
        entity.state["kicks"] += 1
    playing_hud(entity, ctx)
    # Settings opens over the menu (Back and Escape return to it); the
    # menu hides while it's covered.
    if ui.screen("menu") and screens.top == "menu":
        pause_menu(entity, ctx)
    if ui.screen("settings"):
        settings_screen(entity, ctx)
    entity.attributes["paused"] = screens.paused
    entity.attributes["kicks"] = entity.state["kicks"]


def playing_hud(entity, ctx):
    ui = ctx.ui
    if ctx.settings.get("spin_meter", True):
        spin = ctx.scene["Cube"].state.get("speed", 0.0)
        with ui.column(width=260, anchor="top_left", margin=24, gap=8):
            with ui.row(columns=2):
                ui.label("SPIN", size=14, shadow=True, font=PIXEL)
                ui.label(f"{spin:.1f} rad/s", size=14, align="right", shadow=True, font=PIXEL)
            ui.progress(spin, height=14, max=8.0)
            ui.label(f"KICKS {entity.state['kicks']}", size=14, shadow=True, font=PIXEL)
    if ui.button("Menu", *ui.place("top_right", 120, 44, margin=24), 120, 44):
        ui.screens.show("menu", pause=True)
    if not ui.screens.paused:
        w, h = ui.size
        ui.label("Click anywhere to kick the shapes. P, Esc or Start opens the menu.",
                 w / 2, h - 56, size=18, align="center", shadow=True)


def shade(ui):
    """Dims the game behind a screen (and keeps clicks off it)."""
    w, h = ui.size
    ui.panel(0, 0, w, h, color=(0.0, 0.0, 0.0, 0.3))


def pause_menu(entity, ctx):
    ui = ctx.ui
    shade(ui)
    with ui.column(width=380, anchor="center", padding=40, gap=14, panel=True):
        ui.label("PAUSED", size=28, align="center", font=PIXEL)
        ui.label("", size=6)
        if ui.button("Resume", height=52):
            ui.screens.clear()
        if ui.button("Settings", height=52):
            ui.screens.show("settings", pause=True)
        kicks = entity.state["kicks"]
        if ui.button(f"Reset kicks ({kicks})", height=52, id="reset", enabled=kicks > 0):
            entity.state["kicks"] = 0
        ui.label("Arrows or d-pad to choose, Enter or A to press", size=14, align="center",
                 color=ui.theme.text_dim)


def settings_screen(entity, ctx):
    ui = ctx.ui
    settings = ctx.settings
    shade(ui)
    with ui.column(width=440, anchor="center", padding=40, gap=12, panel=True):
        ui.label("SETTINGS", size=28, align="center", font=PIXEL)
        ui.label("", size=4)
        with ui.row(columns=2):
            ui.label("Volume")
            ui.label(f"{round(settings.volume * 100)}%", align="right", color=ui.theme.text_dim)
        volume = ui.slider(settings.volume, step=0.05, id="volume")
        if volume != settings.volume:
            settings.volume = volume

        meter = settings.get("spin_meter", True)
        if ui.toggle("Spin meter", meter) != meter:
            settings["spin_meter"] = not meter

        ui.label("Graphics")
        with ui.row(columns=len(PRESETS), height=40, gap=8):
            for preset in PRESETS:
                # The current choice is shown pressed in (disabled).
                if ui.button(preset.title(), size=17, enabled=settings.graphics != preset):
                    settings.graphics = preset

        ui.label("UI size")
        with ui.row(columns=len(UI_SIZES), height=40, gap=8):
            for name, scale in UI_SIZES:
                if ui.button(name, size=17, enabled=abs(settings.ui_scale - scale) > 1e-6):
                    settings.ui_scale = scale

        ui.label("", size=4)
        if ui.button("Back", height=52):
            ui.screens.back()
