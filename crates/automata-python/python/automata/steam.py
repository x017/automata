"""Steam: achievements, stats, the overlay, and whether the game runs on a
Steam Deck, through the Steamworks API.

Scripts use ``ctx.steam``; code-first games ``automata.steam.client()``.
Both are the same :class:`Steam` object, one per process.

An exported game with ``steam: (app_id: …)`` in its ``build.ron`` starts
Steam before it opens its window (the overlay needs that) from Valve's
``steam_api`` library, which the export copies next to the game. The
library is loaded when the game starts, through Steam's flat C API, so
nothing from the Steamworks SDK is needed to build the engine.

When Steam isn't there (Oberon's Play, a game without a Steam app id, Steam
not running, the browser or Android) everything still works and nothing is
sent: unlocking an achievement logs it and remembers it for the session,
stats keep their values for the session, and :attr:`Steam.available` is
False with the reason in :attr:`Steam.status`.

Calls Steam answers at once (achievements, stats, the player's name) come
straight from Steam's local cache. Changes are sent to Steam's servers in
batches: an unlocked achievement at the end of that frame (so its pop-up
shows), changed stats at most every :data:`STORE_INTERVAL` seconds, and
everything when the game ends.
"""

from __future__ import annotations

import atexit
import ctypes
import os
import sys
import time
from typing import Any, Callable

#: How often changed stats are sent to Steam at most, in seconds. An
#: unlocked achievement is sent at the end of its frame.
STORE_INTERVAL = 60.0

#: ``Steam.open_overlay`` dialogs.
OVERLAY_DIALOGS = ("friends", "community", "players", "settings", "officialgamegroup",
                   "stats", "achievements")

# Interface accessors, newest first: whichever the shipped library has.
_VERSIONS = {
    "UserStats": ("v013", "v012", "v011"),
    "Friends": ("v018", "v017"),
    "Utils": ("v011", "v010"),
    "Apps": ("v009", "v008"),
    "RemoteStorage": ("v016", "v014"),
}

# Callback ids (steam_api_internal.h: k_iSteamFriendsCallbacks = 300, …).
_OVERLAY_ACTIVATED = 331  # GameOverlayActivated_t
_STATS_RECEIVED = 1101  # UserStatsReceived_t

# Callback structures are packed to 4 bytes on Linux and macOS, 8 on Windows.
_PACK = 8 if sys.platform == "win32" else 4


class _CallbackMsg(ctypes.Structure):
    _pack_ = _PACK
    _fields_ = [("user", ctypes.c_int32), ("callback", ctypes.c_int32),
                ("param", ctypes.POINTER(ctypes.c_uint8)), ("size", ctypes.c_int32)]


def library_names() -> tuple[str, ...]:
    """The ``steam_api`` library's file name for this platform."""
    if sys.platform == "win32":
        return ("steam_api64.dll",) if ctypes.sizeof(ctypes.c_void_p) == 8 else ("steam_api.dll",)
    if sys.platform == "darwin":
        return ("libsteam_api.dylib",)
    return ("libsteam_api.so",)


def find_library(folders: list[str] | tuple[str, ...] = ()) -> str | None:
    """The ``steam_api`` library to load: ``$AUTOMATA_STEAM_API`` if set,
    else the first of ``folders`` (then the program's own folder) that has
    it."""
    env = os.environ.get("AUTOMATA_STEAM_API")
    if env:
        return env if os.path.isfile(env) else None
    places = [*folders, os.path.dirname(os.path.abspath(sys.executable))]
    for folder in places:
        for name in library_names():
            path = os.path.join(folder, name)
            if os.path.isfile(path):
                return path
    return None


def on_steam_deck() -> bool:
    """Whether the game runs on a Steam Deck (Steam sets ``SteamDeck=1``
    there)."""
    return os.environ.get("SteamDeck", "") == "1"


class Steam:
    """``ctx.steam``. See the module's notes; every method is safe to call
    whether or not Steam is running."""

    def __init__(self, log: Callable[[str, str], Any] | None = None) -> None:
        self._log = log or _print_log
        self._lib: Any = None
        self._pipe = 0
        self._manual = False
        self._ifaces: dict[str, int] = {}
        #: Why Steam isn't available ("not started" until :func:`init`).
        self.status = "not started"
        self._app_id = 0
        self._stats_ready = True
        self._waiting: list[tuple[str, tuple]] = []  # calls made before stats arrived
        self._store_now = False
        self._dirty = False
        self._last_store = 0.0
        #: True while the Steam overlay is open (pause the game then).
        self.overlay_open = False
        # Offline: what this session unlocked and set.
        self._local_achievements: set[str] = set()
        self._local_stats: dict[str, int | float] = {}
        self._told_offline = False
        self._on_overlay: list[Callable[[bool], Any]] = []

    # --- state ------------------------------------------------------------

    @property
    def available(self) -> bool:
        """Whether Steam is running and the game is connected to it."""
        return self._lib is not None

    @property
    def app_id(self) -> int:
        """The game's Steam app id (0 without Steam)."""
        if self._lib is not None and "Utils" in self._ifaces:
            return int(self._lib.SteamAPI_ISteamUtils_GetAppID(self._ifaces["Utils"]))
        return self._app_id

    @property
    def user_name(self) -> str | None:
        """The player's Steam name (None without Steam)."""
        if self._lib is None or "Friends" not in self._ifaces:
            return None
        raw = self._lib.SteamAPI_ISteamFriends_GetPersonaName(self._ifaces["Friends"])
        return raw.decode("utf-8", "replace") if raw else None

    @property
    def language(self) -> str | None:
        """The language the player chose for the game in Steam, in Steam's
        words (``"english"``, ``"german"``, ``"schinese"`` …), or None."""
        if self._lib is None or "Apps" not in self._ifaces:
            return None
        raw = self._lib.SteamAPI_ISteamApps_GetCurrentGameLanguage(self._ifaces["Apps"])
        return raw.decode("utf-8", "replace") if raw else None

    @property
    def steam_deck(self) -> bool:
        """Whether the game runs on a Steam Deck (works without Steam too)."""
        fn = self._fn("SteamAPI_ISteamUtils_IsSteamRunningOnSteamDeck", "Utils")
        if fn is not None and fn(self._ifaces["Utils"]):
            return True
        return on_steam_deck()

    @property
    def big_picture(self) -> bool:
        """Whether Steam is in Big Picture mode (a TV, a controller)."""
        fn = self._fn("SteamAPI_ISteamUtils_IsSteamInBigPictureMode", "Utils")
        return bool(fn is not None and fn(self._ifaces["Utils"]))

    @property
    def overlay_enabled(self) -> bool:
        """Whether the Steam overlay can open (the player may turn it off)."""
        fn = self._fn("SteamAPI_ISteamUtils_IsOverlayEnabled", "Utils")
        return bool(fn is not None and fn(self._ifaces["Utils"]))

    @property
    def cloud(self) -> bool:
        """Whether Steam Cloud keeps this player's saves: on for the game
        (Auto-Cloud, set up in Steamworks) and in the player's Steam
        settings. False without Steam."""
        if self._lib is None or "RemoteStorage" not in self._ifaces:
            return False
        rs = self._ifaces["RemoteStorage"]
        account = self._fn("SteamAPI_ISteamRemoteStorage_IsCloudEnabledForAccount", "RemoteStorage")
        app = self._fn("SteamAPI_ISteamRemoteStorage_IsCloudEnabledForApp", "RemoteStorage")
        return bool(account is not None and app is not None and account(rs) and app(rs))

    def on_overlay(self, fn: Callable[[bool], Any]) -> None:
        """Calls ``fn(open)`` when the overlay opens (True) or closes."""
        if fn not in self._on_overlay:
            self._on_overlay.append(fn)

    # --- achievements -----------------------------------------------------

    def unlock(self, name: str) -> bool:
        """Unlocks the achievement ``name`` (its API name in Steamworks,
        like ``"ACH_WIN_ONE_GAME"``); Steam shows its pop-up at the end of
        the frame. Unlocking it again does nothing. Returns whether Steam
        knows the achievement (True offline)."""
        name = _name(name)
        if self._lib is None:
            if name not in self._local_achievements:
                self._local_achievements.add(name)
                self._offline(f"achievement {name!r} unlocked")
            return True
        if not self._stats_ready:
            self._waiting.append(("unlock", (name,)))
            return True
        if self.unlocked(name):
            return True
        ok = bool(self._lib.SteamAPI_ISteamUserStats_SetAchievement(self._stats(), _c(name)))
        if ok:
            self._store_now = True
        else:
            self._unknown("achievement", name)
        return ok

    def unlocked(self, name: str) -> bool:
        """Whether the player has the achievement ``name``."""
        name = _name(name)
        if self._lib is None:
            return name in self._local_achievements
        if not self._stats_ready:
            return any(c == ("unlock", (name,)) for c in self._waiting)
        got = ctypes.c_bool(False)
        ok = self._lib.SteamAPI_ISteamUserStats_GetAchievement(self._stats(), _c(name),
                                                               ctypes.byref(got))
        return bool(ok and got.value)

    def clear(self, name: str) -> bool:
        """Takes the achievement ``name`` away again (for testing)."""
        name = _name(name)
        if self._lib is None:
            self._local_achievements.discard(name)
            return True
        ok = bool(self._lib.SteamAPI_ISteamUserStats_ClearAchievement(self._stats(), _c(name)))
        self._store_now |= ok
        return ok

    def progress(self, name: str, current: int, maximum: int) -> bool:
        """Shows Steam's progress pop-up for the achievement ``name``
        ("3 / 10"). It doesn't unlock it: call :meth:`unlock` when done."""
        name = _name(name)
        if self._lib is None:
            self._offline(f"achievement {name!r}: {int(current)} / {int(maximum)}")
            return True
        return bool(self._lib.SteamAPI_ISteamUserStats_IndicateAchievementProgress(
            self._stats(), _c(name), max(0, int(current)), max(0, int(maximum))))

    @property
    def achievements(self) -> list[str]:
        """Every achievement's API name, as set up in Steamworks (empty
        without Steam)."""
        if self._lib is None or not self._stats_ready:
            return []
        stats = self._stats()
        n = int(self._lib.SteamAPI_ISteamUserStats_GetNumAchievements(stats))
        names = (self._lib.SteamAPI_ISteamUserStats_GetAchievementName(stats, i) for i in range(n))
        return [raw.decode("utf-8", "replace") for raw in names if raw]

    # --- stats ------------------------------------------------------------

    def stat(self, name: str, default: int | float = 0) -> int | float:
        """The stat ``name`` (an int or float stat in Steamworks), or
        ``default`` when Steam doesn't have it."""
        name = _name(name)
        if self._lib is None or not self._stats_ready:
            return self._local_stats.get(name, default)
        stats = self._stats()
        i = ctypes.c_int32(0)
        if self._lib.SteamAPI_ISteamUserStats_GetStatInt32(stats, _c(name), ctypes.byref(i)):
            return int(i.value)
        f = ctypes.c_float(0.0)
        if self._lib.SteamAPI_ISteamUserStats_GetStatFloat(stats, _c(name), ctypes.byref(f)):
            return float(f.value)
        return default

    def set_stat(self, name: str, value: int | float) -> bool:
        """Sets the stat ``name``. Returns whether Steam knows it (True
        offline). Steam ignores values outside the stat's limits, and an
        "increment only" stat going down."""
        name = _name(name)
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise TypeError(f"a stat is a number, not {value!r}")
        if self._lib is None:
            self._local_stats[name] = value
            return True
        if not self._stats_ready:
            self._local_stats[name] = value
            self._waiting.append(("set_stat", (name, value)))
            return True
        stats, key = self._stats(), _c(name)
        lib = self._lib
        if isinstance(value, int):
            ok = lib.SteamAPI_ISteamUserStats_SetStatInt32(stats, key, max(-2**31, min(2**31 - 1, value)))
            ok = ok or lib.SteamAPI_ISteamUserStats_SetStatFloat(stats, key, float(value))
        else:
            ok = lib.SteamAPI_ISteamUserStats_SetStatFloat(stats, key, float(value))
            ok = ok or lib.SteamAPI_ISteamUserStats_SetStatInt32(stats, key, int(round(value)))
        if ok:
            self._dirty = True
        else:
            self._unknown("stat", name)
        return bool(ok)

    def add_stat(self, name: str, amount: int | float = 1) -> int | float:
        """Adds ``amount`` to the stat ``name`` and returns the new value."""
        value = self.stat(name) + amount
        self.set_stat(name, value)
        return value

    def store(self) -> bool:
        """Sends changed achievements and stats to Steam now (this happens
        by itself; see the module's notes)."""
        self._store_now = self._dirty = False
        self._last_store = time.monotonic()
        if self._lib is None or not self._stats_ready:
            return self._lib is None
        return bool(self._lib.SteamAPI_ISteamUserStats_StoreStats(self._stats()))

    def reset(self, achievements: bool = True) -> bool:
        """Sets every stat back to zero, and with ``achievements`` takes
        every achievement away too. For testing: players can't undo it."""
        if self._lib is None:
            self._local_stats.clear()
            if achievements:
                self._local_achievements.clear()
            return True
        ok = bool(self._lib.SteamAPI_ISteamUserStats_ResetAllStats(self._stats(), bool(achievements)))
        self.store()
        return ok

    # --- the overlay ------------------------------------------------------

    def open_overlay(self, dialog: str = "achievements") -> bool:
        """Opens the Steam overlay at ``dialog``: one of
        :data:`OVERLAY_DIALOGS`. False without Steam."""
        if dialog not in OVERLAY_DIALOGS:
            raise ValueError(f"there's no overlay dialog {dialog!r} (one of {', '.join(OVERLAY_DIALOGS)})")
        if self._lib is None or "Friends" not in self._ifaces:
            return False
        self._lib.SteamAPI_ISteamFriends_ActivateGameOverlay(self._ifaces["Friends"], _c(dialog))
        return True

    def open_store(self, app_id: int | None = None, add_to_cart: bool = False) -> bool:
        """Opens the overlay at a game's store page (this game's unless
        ``app_id``, such as a DLC's). False without Steam."""
        if self._lib is None or "Friends" not in self._ifaces:
            return False
        self._lib.SteamAPI_ISteamFriends_ActivateGameOverlayToStore(
            self._ifaces["Friends"], int(app_id or self.app_id), 1 if add_to_cart else 0)
        return True

    def open_url(self, url: str, modal: bool = False) -> bool:
        """Opens ``url`` in the overlay's browser. False without Steam."""
        if self._lib is None or "Friends" not in self._ifaces:
            return False
        self._lib.SteamAPI_ISteamFriends_ActivateGameOverlayToWebPage(
            self._ifaces["Friends"], _c(str(url)), 1 if modal else 0)
        return True

    # --- the engine's side ------------------------------------------------

    def _frame(self) -> None:
        """Start of a frame: Steam's events (the overlay, stats arriving)."""
        lib = self._lib
        if lib is None:
            return
        if not self._manual:
            lib.SteamAPI_RunCallbacks()
            return
        lib.SteamAPI_ManualDispatch_RunFrame(self._pipe)
        msg = _CallbackMsg()
        while lib.SteamAPI_ManualDispatch_GetNextCallback(self._pipe, ctypes.byref(msg)):
            try:
                self._callback(msg.callback, msg.param, msg.size)
            finally:
                lib.SteamAPI_ManualDispatch_FreeLastCallback(self._pipe)

    def _callback(self, kind: int, param: Any, size: int) -> None:
        if kind == _OVERLAY_ACTIVATED and size >= 1:
            open_now = bool(param[0])
            if open_now != self.overlay_open:
                self.overlay_open = open_now
                for fn in list(self._on_overlay):
                    try:
                        fn(open_now)
                    except Exception as exc:  # noqa: BLE001 - user code
                        self._log("error", f"steam: on_overlay: {exc!r}")
        elif kind == _STATS_RECEIVED and size >= 12 and not self._stats_ready:
            result = int.from_bytes(bytes(param[8:12]), sys.byteorder, signed=True)
            if result == 1:  # k_EResultOK
                self._stats_ready = True
                waiting, self._waiting = self._waiting, []
                for call, args in waiting:
                    getattr(self, call)(*args)
            else:
                self._log("warn", f"steam: couldn't get this player's stats (result {result})")

    def _flush(self, final: bool = False) -> None:
        """End of a frame: sends what changed (see the module's notes)."""
        if self._lib is None or not self._stats_ready:
            return
        due = self._dirty and time.monotonic() - self._last_store >= STORE_INTERVAL
        if self._store_now or due or (final and self._dirty):
            self.store()

    def shutdown(self) -> None:
        """Sends what changed and disconnects from Steam (at exit by itself)."""
        if self._lib is None:
            return
        try:
            self._flush(final=True)
            self._lib.SteamAPI_Shutdown()
        finally:
            self._lib = None
            self._ifaces = {}
            self.status = "shut down"

    # --- helpers ----------------------------------------------------------

    def _stats(self) -> int:
        return self._ifaces.get("UserStats", 0)

    def _fn(self, name: str, iface: str) -> Any:
        if self._lib is None or iface not in self._ifaces:
            return None
        return getattr(self._lib, name, None)

    def _offline(self, what: str) -> None:
        if not self._told_offline:
            self._told_offline = True
            self._log("info", f"steam: {self.status}; achievements and stats stay in this session")
        self._log("info", f"steam (offline): {what}")

    def _unknown(self, kind: str, name: str) -> None:
        self._log("warn", f"steam: there's no {kind} {name!r} in this game's Steamworks settings")


def _name(name: Any) -> str:
    if not isinstance(name, str) or not name:
        raise ValueError(f"an achievement or stat is named by its API name, not {name!r}")
    return name


def _c(text: str) -> bytes:
    return text.encode("utf-8")


def _print_log(level: str, message: str) -> None:
    print(message, file=sys.stderr if level in ("warn", "error") else sys.stdout)


_CLIENT: Steam | None = None


def client() -> Steam:
    """The process's :class:`Steam` (started or not)."""
    global _CLIENT
    if _CLIENT is None:
        _CLIENT = Steam()
    return _CLIENT


def _declare(lib: Any) -> None:
    """Argument and result types of the functions this module calls."""
    p, b, s, i, u, f = (ctypes.c_void_p, ctypes.c_bool, ctypes.c_char_p, ctypes.c_int,
                        ctypes.c_uint32, ctypes.c_float)
    table: dict[str, tuple[Any, list[Any]]] = {
        "SteamAPI_InitFlat": (i, [ctypes.c_char_p]),
        "SteamAPI_Init": (b, []),
        "SteamAPI_Shutdown": (None, []),
        "SteamAPI_RestartAppIfNecessary": (b, [u]),
        "SteamAPI_RunCallbacks": (None, []),
        "SteamAPI_GetHSteamPipe": (ctypes.c_int32, []),
        "SteamAPI_ManualDispatch_Init": (None, []),
        "SteamAPI_ManualDispatch_RunFrame": (None, [ctypes.c_int32]),
        "SteamAPI_ManualDispatch_GetNextCallback": (b, [ctypes.c_int32, ctypes.POINTER(_CallbackMsg)]),
        "SteamAPI_ManualDispatch_FreeLastCallback": (None, [ctypes.c_int32]),
        "SteamAPI_ISteamUserStats_RequestCurrentStats": (b, [p]),
        "SteamAPI_ISteamUserStats_SetAchievement": (b, [p, s]),
        "SteamAPI_ISteamUserStats_GetAchievement": (b, [p, s, ctypes.POINTER(ctypes.c_bool)]),
        "SteamAPI_ISteamUserStats_ClearAchievement": (b, [p, s]),
        "SteamAPI_ISteamUserStats_IndicateAchievementProgress": (b, [p, s, u, u]),
        "SteamAPI_ISteamUserStats_GetNumAchievements": (u, [p]),
        "SteamAPI_ISteamUserStats_GetAchievementName": (s, [p, u]),
        "SteamAPI_ISteamUserStats_GetStatInt32": (b, [p, s, ctypes.POINTER(ctypes.c_int32)]),
        "SteamAPI_ISteamUserStats_GetStatFloat": (b, [p, s, ctypes.POINTER(f)]),
        "SteamAPI_ISteamUserStats_SetStatInt32": (b, [p, s, ctypes.c_int32]),
        "SteamAPI_ISteamUserStats_SetStatFloat": (b, [p, s, f]),
        "SteamAPI_ISteamUserStats_StoreStats": (b, [p]),
        "SteamAPI_ISteamUserStats_ResetAllStats": (b, [p, b]),
        "SteamAPI_ISteamFriends_GetPersonaName": (s, [p]),
        "SteamAPI_ISteamFriends_ActivateGameOverlay": (None, [p, s]),
        "SteamAPI_ISteamFriends_ActivateGameOverlayToStore": (None, [p, u, i]),
        "SteamAPI_ISteamFriends_ActivateGameOverlayToWebPage": (None, [p, s, i]),
        "SteamAPI_ISteamUtils_GetAppID": (u, [p]),
        "SteamAPI_ISteamUtils_IsOverlayEnabled": (b, [p]),
        "SteamAPI_ISteamUtils_IsSteamInBigPictureMode": (b, [p]),
        "SteamAPI_ISteamUtils_IsSteamRunningOnSteamDeck": (b, [p]),
        "SteamAPI_ISteamApps_GetCurrentGameLanguage": (s, [p]),
        "SteamAPI_ISteamRemoteStorage_IsCloudEnabledForAccount": (b, [p]),
        "SteamAPI_ISteamRemoteStorage_IsCloudEnabledForApp": (b, [p]),
    }
    for iface, versions in _VERSIONS.items():
        for v in versions:
            table[f"SteamAPI_Steam{iface}_{v}"] = (p, [])
    for name, (restype, argtypes) in table.items():
        fn = getattr(lib, name, None)
        if fn is not None:
            fn.restype, fn.argtypes = restype, argtypes


def init(app_id: int | None = None, *, folders: list[str] | tuple[str, ...] = (),
         restart: bool = False, log: Callable[[str, str], Any] | None = None) -> Steam:
    """Connects to Steam (once per process; again does nothing) and
    returns :func:`client`. Call it before the game's window opens, so the
    overlay can draw over it; exported games do.

    ``app_id`` is the game's Steam app id. Started by Steam, the game is
    told it; started any other way (while developing) ``app_id`` stands
    in, as a ``steam_appid.txt`` would. With ``restart`` a game started
    outside Steam asks Steam to start it instead and the process should
    end (:attr:`Steam.status` is ``"restarting through Steam"``).

    ``folders`` are where to look for the ``steam_api`` library (see
    :func:`find_library`)."""
    steam = client()
    if log is not None:
        steam._log = log
    if steam._lib is not None or steam.status not in ("not started",):
        return steam
    if app_id:
        steam._app_id = int(app_id)
    if sys.platform in ("emscripten", "android") or "ANDROID_PRIVATE" in os.environ:
        steam.status = "Steam isn't on this platform"
        return steam
    path = find_library(folders)
    if path is None:
        steam.status = f"no {library_names()[0]} next to the game"
        return steam
    try:
        lib = ctypes.CDLL(path)
    except OSError as exc:
        steam.status = f"couldn't load {path}: {exc}"
        return steam
    _declare(lib)
    launched_by_steam = bool(os.environ.get("SteamAppId") or os.environ.get("SteamGameId"))
    if restart and app_id and not launched_by_steam:
        if lib.SteamAPI_RestartAppIfNecessary(int(app_id)):
            steam.status = "restarting through Steam"
            return steam
    if app_id and not launched_by_steam:
        os.environ["SteamAppId"] = str(int(app_id))
    if hasattr(lib, "SteamAPI_InitFlat"):
        err = ctypes.create_string_buffer(1024)
        result = lib.SteamAPI_InitFlat(err)
        if result != 0:
            reason = err.value.decode("utf-8", "replace") or f"error {result}"
            steam.status = f"Steam didn't start: {reason}"
            return steam
    elif not lib.SteamAPI_Init():
        steam.status = "Steam didn't start (is it running, and does this account own the game?)"
        return steam
    for iface, versions in _VERSIONS.items():
        for v in versions:
            fn = getattr(lib, f"SteamAPI_Steam{iface}_{v}", None)
            ptr = fn() if fn is not None else None
            if ptr:
                steam._ifaces[iface] = ptr
                break
    if "UserStats" not in steam._ifaces:
        steam._log("warn", "steam: this steam_api library has no stats interface the engine knows")
    if hasattr(lib, "SteamAPI_ManualDispatch_Init"):
        lib.SteamAPI_ManualDispatch_Init()
        steam._pipe = int(lib.SteamAPI_GetHSteamPipe())
        steam._manual = True
    steam._lib = lib
    # Older libraries load the player's stats on request (newer ones have
    # them when the game starts); calls made before they arrive wait.
    request = getattr(lib, "SteamAPI_ISteamUserStats_RequestCurrentStats", None)
    if request is not None and "UserStats" in steam._ifaces and steam._manual:
        steam._stats_ready = not request(steam._ifaces["UserStats"])
    steam.status = "running"
    steam._last_store = time.monotonic()
    atexit.register(steam.shutdown)
    steam._log("info", f"steam: running as app {steam.app_id}"
               + (f" for {steam.user_name}" if steam.user_name else ""))
    return steam


def _reset_for_tests() -> None:
    """Forgets the process's Steam (tests only)."""
    global _CLIENT
    if _CLIENT is not None and _CLIENT._lib is not None:
        _CLIENT.shutdown()
    _CLIENT = None
