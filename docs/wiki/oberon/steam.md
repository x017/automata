# Steam

`ctx.steam` connects your game to Steam. It gives you:
- achievements and stats,
- the Steam overlay,
- Steam Cloud for save games,
- button labels that match the player's controller, Steam Deck included.

It's safe to call everywhere. In Oberon's Play, on the web, on Android, or
when Steam isn't running, achievements and stats are kept for that session
and logged to the Console. The game carries on as usual:

```
steam (offline): achievement 'ACH_FIRST_COIN' unlocked
```

So you can write the Steam code now and test it in Play before the game
has a Steam page.

AUTOMATA talks to Steam through Valve's `steam_api` library, the one from
the Steamworks SDK. The export copies it into the game. Nothing needs
compiling, but you do need the SDK (below).

## Set up

1. **An app ID.** Steamworks gives your game one when you create its app
   on the [partner site](https://partner.steamgames.com/). To try things
   out before that, use **480**. That's Spacewar, Valve's test app, which
   every Steam account can run.
2. **The Steamworks SDK.** Download it from the partner site
   (*Steamworks SDK* on the *Getting started* page) and unzip it anywhere.
   Keep it outside your project folder, or AUTOMATA leaves it out of
   builds. Valve's licence lets you ship the library inside your game, but
   AUTOMATA can't ship it for you.
3. **Build Settings.** In **Build ▸ Build Settings…**, fill in **Steam app
   ID** and **Steamworks SDK**. The folder button picks the SDK's folder.
   Both are saved in `build.ron`:

   ```ron
   BuildConfig(
       name: "Coin Run",
       // …
       steam: (
           app_id: 480,
           sdk: "~/sdk/steamworks",   // or $AUTOMATA_STEAMWORKS_SDK
           restart: false,            // true for release builds (below)
       ),
   )
   ```

4. **Build.** Windows, Linux and macOS builds now hold the library
   (`steam_api64.dll`, `libsteam_api.so` or `libsteam_api.dylib`) next to
   your scenes, and `game.json` says `"steam": {"app_id": 480, …}`. A
   missing SDK stops the build with a message saying what's wrong. Web and
   Android builds leave Steam out, and `ctx.steam` works offline there.

**Trying a build.** Start Steam and sign in, then run the exported game
the usual way. The Console (or the terminal) says
`steam: running as app 480 for <your name>`. The overlay needs the game to
be started *by Steam*, so it only appears in games you launch from your
library.

**Release builds.** Set `restart: true`. When a player starts the
executable directly, Steam takes over: it starts the game the proper way
(overlay, ownership check) and the first copy quits. Leave it off while
testing, or the game won't start outside Steam.

## Achievements

Create achievements in Steamworks (*Stats & Achievements ▸ Achievements*)
and publish them. Use their **API names** in scripts:

```python
def update(entity, ctx):
    if picked_up_coin:
        ctx.steam.unlock("ACH_FIRST_COIN")
        coins = ctx.steam.add_stat("coins")
        ctx.steam.progress("ACH_HUNDRED_COINS", coins, 100)
        if coins >= 100:
            ctx.steam.unlock("ACH_HUNDRED_COINS")
```

- **`unlock` is cheap to call again.** An achievement already unlocked
  does nothing, so you can call `unlock` every frame a condition holds.
- **A name Steam doesn't know** logs a warning
  (`there's no achievement 'ACH_FIRTS_COIN' in this game's Steamworks
  settings`) and returns `False`.
- **`progress(name, current, maximum)`** shows Steam's "37 / 100" pop-up.
  It doesn't unlock anything: call `unlock` when the goal is reached.
- **`unlocked(name)`** reads one achievement, and `achievements` lists
  every API name.
- **`clear(name)`** takes an achievement back. Use it to test, not in a
  shipped game.

## Stats

Stats are numbers Steam keeps for each player: coins collected, distance
run, best time. Create them in Steamworks as *INT* or *FLOAT*:

| | |
|---|---|
| `ctx.steam.stat("coins", default=0)` | A stat's value, or `default` when Steam doesn't have it. Offline: what this session set, else `default` |
| `ctx.steam.set_stat("best_time", 41.7)` | Sets a stat. Whole numbers are fine for FLOAT stats |
| `ctx.steam.add_stat("coins", 1)` | Adds to a stat and returns the new value |
| `ctx.steam.store()` | Send everything now (rarely needed) |
| `ctx.steam.reset(achievements=True)` | Back to zero, achievements too. For testing |

**When things reach Steam.** An unlock is sent at the end of its frame,
so the pop-up shows straight away. Changed stats go at most once a minute,
and once more when the game quits, as Valve asks. Calling `set_stat`
every frame is fine.

## The overlay

Players open the overlay with Shift+Tab (or the Steam button). Your game
can open it too:

| | |
|---|---|
| `ctx.steam.open_overlay("achievements")` | The overlay at `"achievements"`, `"friends"`, `"community"`, `"players"`, `"settings"`, `"officialgamegroup"` or `"stats"` |
| `ctx.steam.open_store()` | Your game's store page, or another app's: `open_store(app_id, add_to_cart=True)` for a DLC |
| `ctx.steam.open_url("https://…")` | A web page in the overlay's browser |
| `ctx.steam.overlay_open` | Whether the overlay is showing |
| `ctx.steam.on_overlay(fn)` | Calls `fn(open)` each time it opens or closes |
| `ctx.steam.overlay_enabled` | Whether the player allows the overlay at all |

Pause while the overlay is open, so players don't die while they read:

```python
def update(entity, ctx):
    if ctx.steam.overlay_open:
        return            # the game waits
    ...
```

Without Steam these return `False` and nothing opens.

## Steam Cloud

[Save games](scripting.md#save-games) already live in one folder per
game, so Steam's **Auto-Cloud** can keep them in sync with no code. Set it
up in Steamworks under *App Admin ▸ Cloud*:

1. Turn on Steam Cloud and give the game a quota (1 MB is plenty for JSON
   saves).
2. Add one **root path**:

   | Root | Subdirectory | Pattern | OS | Recursive |
   |---|---|---|---|---|
   | `WinAppDataRoaming` | `Coin Run/saves` | `*.json` | All OSes | No |

   Use your game's name (Build Settings' **Game name**) for `Coin Run`.
3. Add **root overrides**, so a save made on Windows carries on on a Mac
   or a Steam Deck:

   | Original root | OS | New root | Add / Replace path |
   |---|---|---|---|
   | `WinAppDataRoaming` | macOS | `MacAppSupport` | (empty) |
   | `WinAppDataRoaming` | Linux + SteamOS | `LinuxXdgDataHome` | (empty) |

4. Publish the changes.

Only `saves/` is synced. `settings.json` (graphics, volume, keys) stays on
each computer, so a Steam Deck doesn't inherit a desktop PC's "ultra"
graphics.

Steam syncs before the game starts and after it quits. Saves are written
atomically (a temporary file, then a rename), so a sync never finds half
a file. `ctx.steam.cloud` says whether Cloud is on for this game and in
the player's Steam settings. Use it to show "Saves sync with Steam Cloud"
in a menu. A game started outside Steam isn't synced.

## Controllers and Steam Deck

`ctx.input.prompt(name)` gives the label to show for an action or key,
for whatever the player is using right now. See
[Button prompts](scripting.md#button-prompts). On a Steam Deck, and with
Steam's virtual controllers, the labels follow the Deck: A B X Y and
L1 R1 L2 R2.

| | |
|---|---|
| `ctx.steam.steam_deck` | Whether the game runs on a Steam Deck. Works without Steam too |
| `ctx.steam.big_picture` | Whether Steam is in Big Picture mode, which usually means a TV and a controller |

A game that's comfortable on a Deck has text that's readable at 1280×800
(raise [`ui_scale`](scripting.md#scaling-to-any-screen) there) and
supports the controller everywhere, menus included
([gamepad focus](scripting.md#keyboard-and-gamepad-focus)).

## More

| | |
|---|---|
| `ctx.steam.available` | Whether the game is connected to Steam |
| `ctx.steam.status` | Why or why not, in words: `"running"`, `"no libsteam_api.so next to the game"`, `"Steam didn't start (is it running, and does this account own the game?)"`… |
| `ctx.steam.app_id` | The game's app ID |
| `ctx.steam.user_name` | The player's Steam name, or `None` |
| `ctx.steam.language` | The language the player picked for the game in Steam (`"english"`, `"german"`, `"schinese"`…), or `None` |

**Python games without Oberon.** Call `steam.init` before the window
opens. `Game.run` takes care of the rest (events each frame, sending, and
shutting down at exit):

```python
import automata as at
from automata import steam

s = steam.init(480)          # looks for steam_api next to the program
game = at.Game(title="Coin Run")

@game.update
def update(ctx):
    if collected_all:
        s.unlock("ACH_ALL_COINS")

game.run()
```

`steam.init(app_id, folders=[...], restart=False, log=None)` looks for the
library in `folders`, then next to the Python executable.
`$AUTOMATA_STEAM_API` names the file directly. `steam.client()` returns
the same object anywhere in the program.
