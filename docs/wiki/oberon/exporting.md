# Exporting your game

An export turns your project into a game other people can run. They don't
need Oberon, Rust or Python installed. AUTOMATA exports to **Windows**
and **Linux** (64-bit), to **macOS** as one app for Apple Silicon and
Intel Macs ([macOS](#macos) below), and to **Android** as an `.apk` you can
install on a phone ([Android](#android) below; still being polished).

Each export is a folder you can zip and share. You can also turn on
**Archive** to get a ready-made `.zip` (Windows, macOS) or `.tar.gz`
(Linux).

```
build/
  windows/Coin Run/
    Coin Run.exe        ← players double-click this
    game/               ← your scenes, scripts and assets
    runtime/            ← a private Python with the engine inside
  linux/Coin Run/
    Coin Run
    game/
    runtime/
  macos/
    Coin Run.app        ← one app for Apple Silicon and Intel Macs
  Coin Run-0.1.0-windows-x86_64.zip
  Coin Run-0.1.0-linux-x86_64.tar.gz
  Coin Run-0.1.0-macos-universal.zip
  android/
    Coin Run-0.1.0.apk  ← install this on a phone
```

## Build from Oberon

1. Save your scene into the project folder (Ctrl+S).
2. Open **Build ▸ Build Settings…**. Set the game name and version, and
   pick the platforms.
3. Press **Build now**, or **Ctrl+Shift+B** anywhere in the editor.

![The Build page in Settings](../../assets/oberon/build.png)

The build runs in the background, so you can keep working while it runs.
Its progress appears in the status bar and the Console. If anything
fails, the Console opens and lists every problem: a missing script, a
scene that won't load, or a missing template. **Open build folder** shows
the result in your file manager.

| Setting | What it does |
|---|---|
| Game name | Window title, executable name and folder name |
| Version | Used in archive names, e.g. `Coin Run-0.1.0-linux-x86_64.zip` |
| Start scene | The scene the game opens. If empty, the build uses the scene open in Oberon. |
| Output folder | Where builds go, relative to the project (`build`) |
| Android app ID | The app's unique ID on Android, like `com.yourname.coinrun`. Empty: `com.example.<name>`, fine for testing |
| Android version code | A whole number Android compares to decide what's an update. Raise it for each release |
| Android release key | Your signing key for publishing ([Signing](#signing) below). **New** makes one; the folder button picks one you have. Empty: this machine's debug key, for testing |
| Android processors | Which processor types the APK holds code for: `arm64-v8a` (phones and tablets) and `x86_64` (emulators on PCs). Empty: both. `arm64-v8a` alone makes a smaller APK for phones |
| macOS bundle ID | The app's unique ID on macOS, like `com.yourname.coinrun`. macOS keeps the game's permissions and preferences under it. Empty: `com.example.<name>`, fine for testing |
| macOS icon | A square PNG in the project (1024×1024 looks sharpest; 16 to 1024 pixels, a power of two) or an `.icns` file. Empty: the standard app icon |
| Platforms | What **Build** / Ctrl+Shift+B exports. **Build ▸ Build for Linux / Windows / macOS / Android** exports just one. |
| Archive | Also write a `.zip` / `.tar.gz` of each build |
| Touch controls | An on-screen stick and buttons that hold the game's keys on touch screens. Turning it on writes a stick for WASD and a Jump button for Space; change them in `build.ron` ([Touch controls](../guides/touch-controls.md)) |

Settings are saved to `build.ron` in the project folder, so commit it with
your project. Each build leaves out `.oberon/`, `__pycache__/`, version
control folders, virtual environments (under any name) and installed
packages (`site-packages/`), the output folder and `tests/`. To
leave out more, add paths to `exclude` in `build.ron`. A trailing `/`
means a folder:

```ron
BuildConfig(
    name: "Coin Run",
    version: "1.0.0",
    start_scene: Some("level.ron"),
    targets: [Linux, Windows, MacOS, Android],
    archive: true,
    exclude: ["tests/", "art/source/"],
    android: (
        package: "com.yourname.coinrun",
        version_code: 1,
        key: "~/keys/coinrun-release.pem",  // optional: your release key
        abis: ["arm64-v8a"],                // optional: phones only
    ),
    macos: (
        bundle_id: "com.yourname.coinrun",
        icon: "art/icon.png",               // optional
    ),
)
```

## Export templates

An export doesn't compile anything. It copies a prebuilt **export
template** (the game launcher plus a private Python with the engine
installed), then adds your project. That's why a build takes seconds, and
why you can export for Windows from Linux and the other way round.

Templates are downloaded once per engine version. **Build** fetches a
missing template by itself, or you can use **Build ▸ Download Export
Templates** or the **Download** buttons on the Build page. The Linux
template is about 75 MB and the Windows one about 25 MB. The macOS one
is about 100 MB, because it holds Python twice, for Apple Silicon and for
Intel. The Android
template is an APK for phones (`arm64-v8a`) and emulators (`x86_64`).

While the repository is private, the plain download link needs you to be
signed in. Oberon and `automata-cli` try these in order:

1. the public release link;
2. the GitHub CLI, if you've signed in with `gh auth login`;
3. a token in `GH_TOKEN` or `GITHUB_TOKEN` (it needs read access to the
   repository).

If none of them works and you run Oberon from a source checkout
(`cargo run -p oberon`), Build compiles `automata-player` with cargo and
uses it as a **Development** template for your own platform.

The Build page shows each template's state:

- **Ready**: the game ships its own Python. This is what you give to
  players.
- **Development**: the game uses the Python on `PATH`, which must have
  `automata` installed. Oberon sets one up automatically when you run a
  source checkout and no template is installed yet. It's fine for testing
  on your own machine, but don't ship it.
- **Not installed**: Build downloads it the first time you build for
  that platform.

Templates live in `~/.local/share/automata/templates/<version>/` on Linux,
`~/Library/Application Support/automata/templates/<version>/` on macOS and
`%APPDATA%\automata\templates\<version>\` on Windows. You can choose
another folder by setting `AUTOMATA_TEMPLATES`.

## Build from the command line

The same exporter is available as `automata-cli`, the AUTOMATA command
line (it was called `automata-export` before). Use it in CI, or for
code-first games that don't use Oberon:

```bash
cargo build --release -p automata-cli
automata-cli templates download             # once per engine version
automata-cli init my_game --scene level.ron # writes build.ron
automata-cli check my_game                  # validate without building
automata-cli stage my_game --out staged/    # just the game folder
automata-cli stage my_game --zip game.zip   # …and zipped, for the web player
automata-cli build my_game --target all     # or linux / windows / macos / android
```

A code-first game, written in Python with `at.Game` and no scene file,
starts from a script instead of a scene:

```bash
automata-cli init my_game --main main.py
```

The exported game runs `main.py` exactly like `python main.py` would,
with the game folder as the working directory. Load files with paths
relative to that folder.

`stage` writes only the game folder a build would ship: your files minus
the ones left out, plus `game.json`. It doesn't need a template. Use it to
look at what gets shipped, or to package the game yourself. `--zip FILE`
also zips that folder: that `game.zip` is what the web player (in
progress) loads. The same files always give the same zip, byte for byte.

## macOS

**Build ▸ Build for macOS** (or `automata-cli build --target macos`)
writes `build/macos/<name>.app`. You can build it on Linux or Windows too.
The one app runs on Apple Silicon and Intel Macs with macOS 11 (Big Sur)
or later. It holds a Python for each processor, so it's about twice the
size of the Linux build.

```
Coin Run.app/Contents/
  Info.plist              name, bundle ID, version, icon
  MacOS/Coin Run          the launcher (Apple Silicon and Intel)
  MacOS/python3-arm64     Python, one per processor
  MacOS/python3-x86_64
  Resources/game/         your scenes, scripts and assets
  Resources/runtime-arm64/   Python's libraries and the engine
  Resources/runtime-x86_64/
  Resources/AppIcon.icns  from macos.icon
```

**Share the `.zip`**, not the folder. Turn on **Archive** for
`Coin Run-0.1.0-macos-universal.zip`. The zip keeps the files' "executable"
flags, which a copy through Windows or a USB stick can lose. When you
build on a Mac, you can also zip the app yourself (Finder ▸ Compress) or
make a disk image with `hdiutil`.

**Logs.** A game started from Finder writes its output to
`~/Library/Logs/<name>/last_run.log`. Console.app lists it under *Log
Reports*. Run from Terminal (`"Coin Run.app/Contents/MacOS/Coin Run"`),
it also prints there. Saves and settings go to
`~/Library/Application Support/<game>/`. Nothing is written inside the
app, so a signed app stays valid.

### Signing and notarisation

macOS checks where an app comes from before it opens it. The exported app
isn't signed by a developer, so on someone else's Mac macOS refuses to open
it at first:

- **Testing, or sharing with friends.** The player opens the app once,
  sees the warning and closes it. Then in **System Settings ▸ Privacy &
  Security** they click **Open Anyway**. On macOS 14 and earlier,
  right-click the app ▸ **Open** works too. Or in Terminal:
  `xattr -dr com.apple.quarantine "Coin Run.app"`.
- **Publishing.** Sign the app with an Apple **Developer ID** and have
  Apple *notarise* it. Then it opens with a double-click like any other
  app. This needs a membership in the Apple Developer Program and a Mac
  with Xcode's command line tools (`xcode-select --install`).

The steps, on a Mac:

1. In Xcode ▸ Settings ▸ Accounts ▸ Manage Certificates, create a
   **Developer ID Application** certificate. `security find-identity -v -p
   codesigning` shows its full name.
2. Sign the app with
   [`scripts/sign_macos.sh`](https://github.com/x017/automata/blob/main/scripts/sign_macos.sh)
   from the AUTOMATA repository:

   ```bash
   scripts/sign_macos.sh "build/macos/Coin Run.app" "Developer ID Application: Your Name (TEAMID)"
   ```

   It signs every native file inside the app, innermost first. Python's
   libraries are in `Contents/Resources`, where `codesign --deep` doesn't
   look. Then it signs the app, which seals the game files. It turns on the
   *hardened runtime* and adds a secure timestamp, which notarisation
   requires, and the one exception Python needs
   ([`macos.entitlements`](https://github.com/x017/automata/blob/main/scripts/macos.entitlements)):
   memory for `ctypes` callbacks. With `-` as the identity it signs "ad
   hoc", which is fine for your own Macs only.
3. Zip the *signed* app and send it to Apple. Notarisation takes a few
   minutes. Store your credentials once with an
   [app-specific password](https://support.apple.com/102654):

   ```bash
   xcrun notarytool store-credentials automata --apple-id you@example.com --team-id TEAMID
   ditto -c -k --keepParent "build/macos/Coin Run.app" "Coin Run.zip"
   xcrun notarytool submit "Coin Run.zip" --keychain-profile automata --wait
   ```

   If it says `Invalid`, `xcrun notarytool log <id> --keychain-profile
   automata` lists each problem.
4. Staple the ticket to the app, so it opens offline too. Then check it,
   and zip it again for players:

   ```bash
   xcrun stapler staple "build/macos/Coin Run.app"
   spctl --assess --type execute --verbose "build/macos/Coin Run.app"
   ditto -c -k --keepParent "build/macos/Coin Run.app" "Coin Run-1.0.0-macos.zip"
   ```

Sign after every build: a new build replaces the app, signature and all.
Once the app is signed, don't change anything inside it. That includes the
game files and the `Info.plist`, because the signature covers them.

## Android

**Build ▸ Build for Android** (or `automata-cli build --target android`)
writes `build/android/<name>-<version>.apk`. Like the desktop builds it
needs no compiler, and no Android SDK, NDK or Java either. The build
copies the Android template and then:

- adds your game's files to the APK;
- gives the app your game's name, **Android app ID** and version;
- aligns and signs the APK.

Install it on a phone with `adb install "build/android/Coin Run-0.1.0.apk"`,
or copy the file over and open it. Android asks you to allow installs from
that app first.

The first time the game starts, it unpacks Python and its files into the
app's storage, which takes a moment. After that it starts straight away,
until an update brings different files. Saves and settings stay through
updates.

**Size.** Python, the engine and the player add about 16.5 MB per
processor type to your game's files. The template holds two:
`arm64-v8a` for phones and tablets, and `x86_64` for emulators. So a build
with both comes to about 34 MB plus the game, and one with just
**Android processors** set to `arm64-v8a` comes to about 17 MB plus the game.
To save space, the Android build of Python leaves out parts of the standard
library that a game on a phone can't use: the interactive tools (REPL, `pdb`,
`pydoc`, `doctest`, `unittest`, profilers), `wsgiref`, `xmlrpc`, `dbm`, the
mail clients (`smtplib`, `imaplib`, ...), and the CJK codecs (`shift_jis`,
`gbk`, `big5`, ...). `bz2` and `lzma` aren't included either. Everything else
is there: `json`, `random`, `re`, `dataclasses`, `asyncio`, `sqlite3`, `ssl` and
`urllib`/`http`, `zipfile`/`gzip`, `xml`, `csv`, `logging`, and the rest.

### Signing

Android only installs signed apps, and only accepts an update signed by
the same key.

**Debug key (testing).** Without a release key, the first Android build
makes a *debug key* for your machine, like Android Studio does, and every
build after that uses it. It's saved as `android-debug.key` in
`~/.config/automata/` (Linux, macOS) or `%APPDATA%\automata\` (Windows),
or wherever `AUTOMATA_ANDROID_KEY` points. Builds on another machine
won't install over yours unless you copy the key there too. It's fine for
your own phones and for testers.

**Release key (publishing).** To publish the game, sign it with a key of
your own. Make one in **Settings ▸ Build ▸ Android release key ▸ New**, or
on the command line:

```bash
automata-cli keygen ~/keys/coinrun-release.pem --name "Your Name"
```

That writes a 2048-bit RSA key and a self-signed certificate with your
name, valid until 2076, into one PEM file that only you can read. Point
`android.key` in `build.ron` at it (Oberon's **New** does that for you),
or pass `--key FILE` to `automata-cli build`, or set
`AUTOMATA_ANDROID_RELEASE_KEY` (handy for a CI secret). Relative paths
are from the project folder and `~/` is your home folder. The key must
live *outside* the project folder, so it can't end up in the game or in
version control. A build refuses a key inside it.

Back the key up somewhere safe. Every update of your game has to be signed
with it, and a lost key can't be replaced: players would have to uninstall
and install the game again.

**A key you already have** works too, for example from Android Studio or
an earlier release. Its certificate comes with it, so updates install over
the versions you've already published. Convert the keystore to an
unencrypted PEM file once:

```bash
keytool -importkeystore -srckeystore release.jks -destkeystore release.p12 -deststoretype PKCS12
openssl pkcs12 -in release.p12 -nodes -out release.pem
```

PEM files with `BEGIN PRIVATE KEY` (PKCS #8) or `BEGIN RSA PRIVATE KEY`
(PKCS #1) keys, with or without a certificate, all work. Keys with a
password don't yet (decrypt them as above), and only RSA keys of 2048
bits or more are accepted.

**Stores.** The `.apk` installs directly and suits stores and sites that
take APKs (itch.io, F-Droid-style repositories, sideloading). Google Play
wants new apps as an Android App Bundle (`.aab`), which AUTOMATA can't
make yet.

**App ID.** Choose it before anyone installs your game. Android treats a
different ID as a different app. Use letters, digits and `_`, in at least
two parts separated by dots (`com.yourname.coinrun`).

On a phone the game fills the screen in the shape of its resolution
(landscape for 1280×720). Touch controls are set up under
[Touch controls](../guides/touch-controls.md).

## Running an exported game

Players start the executable. It also accepts two flags, which help with
smoke tests in CI:

| Flag | Effect |
|---|---|
| `--headless` | Run without a window |
| `--frames N` | Quit after N frames |

```bash
"build/linux/Coin Run/Coin Run" --headless --frames 60
"build/macos/Coin Run.app/Contents/MacOS/Coin Run" --headless --frames 60
```

Each run writes its output and any Python traceback to
`logs/last_run.log` next to the executable (on macOS, to
`~/Library/Logs/<name>/last_run.log`). On Windows, the game has no
console window, so when it crashes the log opens in Notepad.

## How it works

`automata-player` is a small native launcher written in Rust. It finds
`runtime/` and starts its Python with `python -m automata.player game/`.
In a macOS app it uses the `runtime-<processor>/` for the Mac it runs on,
and the matching `python3-<processor>` in `Contents/MacOS`. Because that
Python sits inside the app, macOS treats the game as the app itself, with
its name, icon and Dock tile.
The player reads `game/game.json`, which holds the name, the start scene
or main script, and the resolution, then runs the game. Scenes run through
`at.run_scene` and scripts through `runpy`. Templates come from the
[Export templates workflow](https://github.com/x017/automata/actions/workflows/export.yml)
on GitHub. It builds the launcher and the engine wheel on Windows,
Linux, and macOS (on Apple Silicon and Intel runners), installs them into
a portable CPython 3.12, smoke-tests an export of *Coin Run*, and
publishes the results to the `export-templates` release. The two macOS
halves are merged into one template: a universal launcher (`lipo`) plus
both Pythons. That template's *Coin Run* is tested natively, under
Rosetta, unzipped, and after `sign_macos.sh`.

The Android template comes from the
[Android workflow](https://github.com/x017/automata/actions/workflows/android.yml).
python-for-android builds an APK with Python, the engine and the player,
but no game. The workflow then builds *Coin Run* from it with
`automata-cli`, plays it on an Android emulator, and publishes the
template. An Android build puts the game in the APK under
`assets/automata/game/`. It changes the app ID, name and version in the
APK's compiled manifest and resource table, and signs the APK with APK
Signature Scheme v2.
