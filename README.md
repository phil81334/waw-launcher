# WaW Launcher

**An unofficial community app. Not affiliated with Treyarch, Activision or UGX.**

The original World at War Modtools Launcher workflows with a new UI. Open the editors, build mods, compile levels and run the game. Made by [phil81334](https://github.com/phil81334), using JavaScript, Rust, Vite and Tauri.

It runs your installed Modtools executables with the original commands and stage order. It does not replace the compilers, sound converter or IWD archiver. Confirmed original bugs are fixed: broken options, failed builds publishing old files, unsafe map deletion and settings losing comma values.

## Download and run

1. Install World at War and its Modtools in the same game folder.
2. Download **waw-launcher.exe** from [Releases](https://github.com/phil81334/waw-launcher/releases). Release users do not need development tools.
3. Put it beside `CoDWaW.exe`, then open it. Keep the original tools in `bin`.

Windows needs [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/#download-section), usually already installed. If missing, install its **Evergreen Bootstrapper**.

## Use it

- **Compile Level:** choose a map, check the stages you want, then compile. BSP and lighting options are beside their stages.
- **Mod Builder:** choose a mod, select its IWD files, edit `mod.csv` if needed, then build the checked stages.
- **Run Game:** choose SP or MP, a mod and any launch options.
- **Use companion apps:** switches editor/viewer shortcuts to the matching WaW family apps in the game folder. Build tools still use the original Modtools. Missing apps are reported; nothing is downloaded.

Settings use the original `bin/Launcher/map_settings` folder. Mod file selections use `<mod>/<mod>.files`. Successful builds publish eligible FF/IWD/arena files to WaW's Local AppData folders, as the original Launcher does. Sound building processes the installation's aliases, not just the selected mod.

Create Map uses your installed original templates. Some original MP templates reference missing scripts or CSVs: supply those inputs yourself; this app does not install patches or invent map content. Delete Map shows its file list before deleting. Cancel stops this Launcher's current job, not unrelated game/editor windows.

## Run from source

**Windows setup — once.** Skip anything already installed.

| Install | Choose |
| --- | --- |
| [VS Code](https://code.visualstudio.com/) | Windows installer. |
| [Node.js](https://nodejs.org/en/download) | LTS, Windows installer. npm is included. |
| [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) | **Desktop development with C++**, including its MSVC tools and Windows SDK. |
| [Rust](https://rust-lang.org/tools/install/) | **rustup-init.exe (x64)**, default installation. |

Close and reopen VS Code after installing these. [Tauri's Windows guide](https://v2.tauri.app/start/prerequisites/#windows) has more help. No Python is needed.

1. Download and extract this repository's ZIP, or clone it.
2. In VS Code, open the folder containing `package.json`.
3. Choose **Terminal → New Terminal** and run:

   ```powershell
   npm install
   npm run dev
   ```

The first desktop build takes longer while Rust dependencies compile. Keep the terminal open; **Ctrl+C** stops development. Next time, click **dev** in VS Code's **NPM Scripts** panel. If hidden, right-click the Explorer panel headings and enable **NPM Scripts**.

Development defaults to the empty `.local/test-root` folder so it cannot accidentally build into your game. To use a prepared Modtools copy, create `.local/game-path.txt` and paste that folder's full address into it: one line, no quotes or EXE filename. Use your own path on whichever drive holds the copy. Only choose your real installation if you want development actions to change it. Released EXEs always use their own folder.

| Command | What it does |
| --- | --- |
| `npm run dev` | Opens the desktop app for development. |
| `npm run dev:web` | Previews the UI in a browser. Cannot run tools or change game files. |
| `npm run build` | Creates `release/waw-launcher.exe`. |
| `npm run build:web` | Builds UI files only. No EXE. |

[Credits](ATTRIBUTION.md) · [GPL-3.0 licence](LICENCE)

