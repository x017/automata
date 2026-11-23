"""A pause menu and a settings screen, drawn with ``ctx.ui``.

The game starts paused, on the menu. Resume (or P) plays: the shapes turn,
and a click anywhere in the game gives them a kick. Clicks on the HUD's
Menu button belong to the UI, so they never kick. Settings changes the
volume and graphics through ``ctx.settings``, which keeps them between runs,
along with a "Spin meter" toggle stored there too.

Titles and the HUD's numbers use a pixel font from the project's
``fonts/`` folder; everything else keeps the built-in font.
"""

#: Press Start 2P (SIL Open Font License, see fonts/PressStart2P-OFL.txt).
PIXEL = "fonts/PressStart2P-Regular.ttf"
PANEL_W = 380
BUTTON_W, BUTTON_H = 300, 52
PRESETS = ("low", "medium", "high", "ultra")


def start(entity, ctx):
    entity.state["screen"] = "menu"  # "menu", "settings", or None: playing
    entity.state["kicks"] = 0
    ctx.settings.bind("pause", "p")


def update(entity, ctx):
    s = entity.state
    if ctx.input.just_pressed("pause"):
        s["screen"] = None if s["screen"] else "menu"
    if s["screen"] is None:
        # ctx.input never sees presses that began on the UI.
        if ctx.input.mouse_just_pressed("left"):
            s["kicks"] += 1
        playing_hud(entity, ctx)
    elif s["screen"] == "menu":
        pause_menu(entity, ctx)
    else:
        settings_screen(entity, ctx)
    entity.attributes["paused"] = s["screen"] is not None
    entity.attributes["kicks"] = s["kicks"]


def centred(ctx, height):
    """The top-left of a panel ``PANEL_W`` wide, centred on the screen."""
    w, h = ctx.hud.size
    return (w - PANEL_W) / 2, (h - height) / 2


def playing_hud(entity, ctx):
    ui = ctx.ui
    w, h = ctx.hud.size
    if ctx.settings.get("spin_meter", True):
        spin = ctx.scene["Cube"].state.get("speed", 0.0)
        ui.label("SPIN", 24, 22, size=14, shadow=True, font=PIXEL)
        ui.label(f"{spin:.1f} rad/s", 284, 22, size=14, align="right", shadow=True, font=PIXEL)
        ui.progress(spin, 24, 46, 260, 14, max=8.0)
        ui.label(f"KICKS {entity.state['kicks']}", 24, 74, size=14, shadow=True, font=PIXEL)
    if ui.button("Menu", w - 144, 20, 120, 44):
        entity.state["screen"] = "menu"
    ui.label("Click anywhere to kick the shapes. P opens the menu.", w / 2, h - 44,
             size=18, align="center", shadow=True)


def pause_menu(entity, ctx):
    ui = ctx.ui
    x, y = centred(ctx, 300)
    ui.panel(x, y, PANEL_W, 300)
    ui.label("PAUSED", x + PANEL_W / 2, y + 28, size=28, align="center", font=PIXEL)
    bx = x + (PANEL_W - BUTTON_W) / 2
    if ui.button("Resume", bx, y + 82, BUTTON_W, BUTTON_H):
        entity.state["screen"] = None
    if ui.button("Settings", bx, y + 146, BUTTON_W, BUTTON_H):
        entity.state["screen"] = "settings"
    kicks = entity.state["kicks"]
    if ui.button(f"Reset kicks ({kicks})", bx, y + 210, BUTTON_W, BUTTON_H, id="reset",
                 enabled=kicks > 0):
        entity.state["kicks"] = 0


def settings_screen(entity, ctx):
    ui = ctx.ui
    settings = ctx.settings
    x, y = centred(ctx, 400)
    left, inner = x + 40, PANEL_W - 80
    ui.panel(x, y, PANEL_W, 400)
    ui.label("SETTINGS", x + PANEL_W / 2, y + 28, size=28, align="center", font=PIXEL)

    ui.label("Volume", left, y + 84)
    ui.label(f"{round(settings.volume * 100)}%", left + inner, y + 84, align="right",
             color=ui.theme.text_dim)
    volume = ui.slider(settings.volume, left, y + 114, inner, 26, step=0.05, id="volume")
    if volume != settings.volume:
        settings.volume = volume

    meter = settings.get("spin_meter", True)
    if ui.toggle("Spin meter", meter, left, y + 162) != meter:
        settings["spin_meter"] = not meter

    ui.label("Graphics", left, y + 212)
    gap = 8
    bw = (inner - gap * (len(PRESETS) - 1)) / len(PRESETS)
    for i, preset in enumerate(PRESETS):
        current = settings.graphics == preset
        # The current preset is shown pressed in (disabled).
        if ui.button(preset.title(), left + i * (bw + gap), y + 242, bw, 40, size=17,
                     enabled=not current):
            settings.graphics = preset

    if ui.button("Back", x + (PANEL_W - BUTTON_W) / 2, y + 318, BUTTON_W, BUTTON_H):
        entity.state["screen"] = "menu"
