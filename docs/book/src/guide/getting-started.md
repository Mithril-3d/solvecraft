# Getting started

SolidCraft keeps its compatibility names: the desktop executable is `solvecraft`, the CLI
is `solvecraft-cli`, and saved designs remain `.solvecraft`. Published 0.2.0 and 0.2.1 packages
were released under the SolveCraft name.

## Desktop

Download the latest build from the
[releases page](https://github.com/bherbruck/solidcraft/releases), or build it yourself with
stable Rust (1.95 or newer):

```sh
git clone https://github.com/bherbruck/solidcraft && cd solidcraft
cargo run --release -p solvecraft -- --sample   # opens with a sample part
```

`solvecraft part.step` opens a STEP, IGES, 3MF, STL or OBJ file as a new design, and
`solvecraft design.solvecraft` opens a saved design. Windows builds cross-compile from Linux with
`cargo xwin build --release --target x86_64-pc-windows-msvc -p solvecraft`.

### Linux Flatpak

Install Flatpak using your distribution's package manager. For an installation that receives
updates, add Flathub for the required Freedesktop runtime and the SolidCraft application feed
to the same user installation:

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak remote-add --user --if-not-exists solvecraft https://bherbruck.github.io/solidcraft/flatpak/solvecraft.flatpakrepo
flatpak install --user solvecraft ai.storyteller.solvecraft
flatpak run ai.storyteller.solvecraft
```

The application ID and remote name retain their compatibility names. To update an existing
remote that still uses the former website address, run
`flatpak remote-modify --user --url=https://bherbruck.github.io/solidcraft/flatpak/repo/ solvecraft`
and then `flatpak update --user`.

Alternatively, download the release's `.flatpak` file for your architecture and run
`flatpak install --user ./solvecraft-VERSION-linux-ARCH.flatpak`. The bundle supplies the
Flathub runtime repository address; installation still needs internet access to fetch any
missing runtime. Launch with `flatpak run ai.storyteller.solvecraft`, rather than looking for
a host `solvecraft` binary. Files chosen through Open/Save use the desktop file-chooser portal;
command-line paths outside Documents may need a specific additional filesystem permission.

## Web

The same app runs in the browser: [open SolidCraft on the web](../app/) (`?sample` opens the sample
part, `?webgl` forces WebGL2 where WebGPU is missing). Designs are saved in the browser's private
file storage, which needs HTTPS or localhost. To build it yourself:

```sh
rustup target add wasm32-unknown-unknown
cd apps/solvecraft-web && trunk serve --release
```

## CLI

`solvecraft-cli` runs the same commands without a window:

```sh
cargo run --release -p solvecraft-cli -- commands                       # every command, as JSON
cargo run --release -p solvecraft-cli -- run examples/bracket.json --out bracket.step
cargo run --release -p solvecraft-cli -- eval examples/bracket.json     # volume, area, faces…
cargo run --release -p solvecraft-cli -- snapshot examples/bracket.json --out bracket.png
cargo run --release -p solvecraft-cli -- exec solid.box '{"length": 20, "width": 10, "height": 5}'
```

A script is a list of commands:

```json
{"commands": [
  {"command": "sketch.create", "params": {"plane": "XY"}},
  {"command": "sketch.rectangle.two_point", "params": {"p0": [0, 0], "p1": [40, 30]}},
  {"command": "sketch.finish", "params": {}},
  {"command": "solid.extrude", "params": {"distance": 20}},
  {"command": "solid.fillet", "params": {"edges": [[0, 0, 10]], "radius": 3}}
]}
```

![The bracket example, rendered by solvecraft-cli snapshot](../generated/bracket.png)

Units are millimetres and degrees unless an expression says otherwise (`"1 in"`, `"30 deg"`),
and Z is up.
