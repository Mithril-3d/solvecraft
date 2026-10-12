# SolidCraft with Nix

The flake supports `x86_64-linux` and `aarch64-linux`. Enable Nix's `nix-command`
and `flakes` experimental features. The committed `flake.lock` pins nixpkgs and
its Rust toolchain; package evaluation checks that compiler against the Rust
floor in `Cargo.toml`.

## Run, build or install

Run the current app without installing it:

```sh
nix run github:bherbruck/solidcraft#solidcraft -- --sample
```

Inside a checkout, `nix run .` and `nix run .#solidcraft` launch the desktop app.
`nix run .#cli -- --version` launches the CLI. The legacy `.#solvecraft` app name
also works. These commands use the flake's exact wrapped binary, rather than an
older executable already on your shell's `PATH`.

Build without installing:

```sh
nix build github:bherbruck/solidcraft#solidcraft
./result/bin/solidcraft --version
./result/bin/solidcraft --sample
```

The result contains `solidcraft`, the compatible `solvecraft` binary name, and
`solvecraft-cli`. **Building does not install a desktop-menu entry.** For a user
profile installation, run:

```sh
nix profile install github:bherbruck/solidcraft#solidcraft
solidcraft --sample
```

This installs the SolidCraft desktop entry, MIME definition and generated icon
sizes alongside the commands. NixOS/Home Manager can instead put the flake's
`packages.${system}.solidcraft` in `environment.systemPackages`/`home.packages`.
If a non-NixOS desktop does not discover profile-installed launchers, ensure that
its session includes `~/.nix-profile/share` in `XDG_DATA_DIRS`, then log out and
back in. The existing `ai.storyteller.solvecraft` desktop/application ID and
`.solvecraft` design files remain compatible.

Check which executable a plain terminal command would run:

```sh
type -a solidcraft solvecraft
readlink -f "$(command -v solidcraft)"
solidcraft --version
nix run github:bherbruck/solidcraft#solidcraft -- --version
```

An unrelated old `solvecraft` executable on `PATH` is not updated by `nix build`.
Use the flake app or `./result/bin/solidcraft` to verify the package you just built.

## Graphics and file dialogs

The app wrapper includes Wayland, X11, xkbcommon, EGL, Vulkan-loader and D-Bus
libraries. It exposes `/run/opengl-driver/lib` and
`/run/opengl-driver/share` so NixOS's graphics drivers and Vulkan manifests are
discoverable. Enable `hardware.graphics.enable = true` on NixOS and keep the
system's driver stack consistent with the compositor. File pickers also need a
working session D-Bus and the desktop's `xdg-desktop-portal` service.

On another distribution, graphics drivers outside the Nix store may need a
matching [nixGL wrapper](https://github.com/nix-community/nixGL). Follow its
GPU-specific instructions; a wrapper for a different driver is not a substitute.
For Intel/Mesa, after building:

```sh
nix run github:nix-community/nixGL#nixGLIntel -- ./result/bin/solidcraft --sample
```

If a window is black, first verify that you launched the newly wrapped package
and that your system exposes a functioning GPU driver. `WGPU_BACKEND=vulkan`
selects Vulkan for diagnosis; a loader without a usable driver still cannot render.
OpenGL rendering uses 1x anti-aliasing to avoid the multisampled black-frame failure.
Vulkan and other backends keep the requested sample count; saved preferences are preserved.
For older builds, including 0.3.0, set Anti-aliasing to 1x and restart when using OpenGL.
If that window cannot show the settings dialog, edit only the `msaa` field to `1` in
`~/.local/share/solvecraft/graphics.json`; preserve any other settings.

## Development shells

`nix develop` supplies runtime libraries and uses the Rust/C toolchain already on
your `PATH`. `nix develop .#rust` supplies the pinned toolchain, C linker and
`target/nix` build directory. Both retain the GUI and CLI's existing Cargo package
names (`solvecraft`, `solvecraft-cli`).
