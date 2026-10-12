{
  description = "SolidCraft — native app development shell and package for Linux";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # Why this shell exists. winit opens the window by dlopen'ing the Wayland client libraries,
      # wgpu renders through the EGL loader (libEGL.so.1), and rfd's file dialogs reach
      # xdg-desktop-portal through libdbus-1; a plain `cargo run` under NixOS finds none of them,
      # because there is no /usr/lib and nothing puts them on LD_LIBRARY_PATH:
      #   winit EventLoopError: The wayland library could not be loaded
      #   eframe: WGPU error: Failed to create surface for any enabled backend: {}
      #   rfd::backend::xdg_desktop_portal::portal::libdbus: Can't connect to a portal: libdbus-1.so not found
      # The Mesa drivers themselves — and the EGL vendor / Vulkan ICD descriptions that name them —
      # come from the system in /run/opengl-driver (hardware.graphics.enable), not from here, since
      # that is the driver stack the compositor runs on. Verified on an Intel LNL iGPU under
      # Wayland: wgpu picks the GL backend through this loader and draws.
      # The X11 libraries are the ones the app actually uses on a Wayland desktop: with `DISPLAY`
      # set and `SOLVECRAFT_WAYLAND` unset, winit runs through XWayland (file drag and drop exists
      # only there). The Wayland ones are for `SOLVECRAFT_WAYLAND=1`, and both are needed under
      # `xvfb-run` on a machine with no display at all.
      runtimeLibs =
        pkgs:
        (with pkgs; [
          wayland
          libxkbcommon
          libglvnd
          vulkan-loader
          dbus
        ])
        ++ (with pkgs; [
          # xorg.libX11 and friends are deprecated aliases in current nixpkgs.
          libx11
          libxcursor
          libxi
          libxrandr
          libxrender
          libxext
          libxcb
        ]);
      driverLibDir = "/run/opengl-driver/lib";
      driverShareDir = "/run/opengl-driver/share";
      libraryPath = pkgs: "${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}:${driverLibDir}";

      shellHook = ''
        if [ ! -d ${driverLibDir} ]; then
          echo "SolidCraft: ${driverLibDir} is missing — on NixOS enable hardware.graphics; on other distributions use a matching nixGL driver wrapper." >&2
        fi
        command -v cc >/dev/null || echo "solvecraft: no C linker on PATH, cargo cannot link — use nix develop .#rust" >&2
        echo "solvecraft: cargo run --release -p solvecraft -- --sample"
        export XDG_DATA_DIRS="${driverShareDir}:''${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
      '';
    in
    {
      devShells = forAllSystems (pkgs: {
        # The shell to work in. It brings no toolchain of its own, Rust or C: the ones already on
        # PATH (a system package, rustup, nixpkgs) keep their build cache and their linker, where a
        # second rustc — or mkShell's second gcc, with another glibc behind it — would make every
        # `cargo build` produce binaries from somewhere else.
        default = pkgs.mkShellNoCC {
          name = "solidcraft";
          LD_LIBRARY_PATH = libraryPath pkgs;
          inherit shellHook;
        };

        # For a machine with no Rust at all: `nix develop .#rust`. Its own target directory, so
        # switching between this toolchain and the system one does not thrash the cache.
        rust = pkgs.mkShell {
          name = "solidcraft-rust";
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
          ];
          LD_LIBRARY_PATH = libraryPath pkgs;
          # Under target/, which .gitignore already covers.
          CARGO_TARGET_DIR = "target/nix";
          shellHook = ''
            ${shellHook}
            echo "solvecraft: $(rustc --version), building into target/nix"
          '';
        };
      });

      packages = forAllSystems (pkgs: rec {
        default = solidcraft;
        solidcraft = assert pkgs.lib.assertMsg
          (pkgs.lib.versionAtLeast pkgs.rustc.version (pkgs.lib.importTOML ./Cargo.toml).workspace.package.rust-version)
          "SolidCraft requires the Rust version advertised in Cargo.toml; update the pinned nixpkgs input.";
          pkgs.rustPlatform.buildRustPackage {
          pname = "solidcraft";
          version = (pkgs.lib.importTOML ./Cargo.toml).workspace.package.version;
          src = self;

          cargoLock.lockFile = ./Cargo.lock;
          # The CLI draws the same packaging icons as the other Linux distributions.
          cargoBuildFlags = [
            "-p"
            "solvecraft"
            "-p"
            "solvecraft-cli"
          ];
          cargoInstallFlags = [
            "-p"
            "solvecraft"
            "-p"
            "solvecraft-cli"
          ];
          # `cargo xtask ci` is the test gate (it wants a wasm target and a GPU); the sandbox has
          # neither.
          doCheck = false;

          nativeBuildInputs = [ pkgs.makeWrapper ];
          buildInputs = runtimeLibs pkgs;

          postInstall = ''
            install -Dm644 packaging/linux/ai.storyteller.solvecraft.desktop \
              $out/share/applications/ai.storyteller.solvecraft.desktop
            install -Dm644 packaging/linux/ai.storyteller.solvecraft.mime.xml \
              $out/share/mime/packages/ai.storyteller.solvecraft.xml
            for size in 16 24 32 48 64 128 256 512; do
              icon=$out/share/icons/hicolor/''${size}x''${size}/apps/ai.storyteller.solvecraft.png
              mkdir -p "$(dirname "$icon")"
              $out/bin/solvecraft-cli icon --size "$size" --out "$icon" >/dev/null
            done
            ln -s solvecraft $out/bin/solidcraft
          '';

          # The app reaches the window through dlopen'd libraries, so the packaged binary needs the
          # same LD_LIBRARY_PATH the shell exports.
          postFixup = ''
            wrapProgram $out/bin/solvecraft \
              --prefix LD_LIBRARY_PATH : "${libraryPath pkgs}" \
              --set-default XDG_DATA_DIRS /usr/local/share:/usr/share \
              --prefix XDG_DATA_DIRS : "$out/share:${driverShareDir}"
          '';

          meta = {
            description = "Rust-native parametric 3D CAD app";
            homepage = "https://github.com/bherbruck/solidcraft";
            license = with pkgs.lib.licenses; [
              mit
              asl20
            ];
            mainProgram = "solidcraft";
          };
        };
      });

      apps = forAllSystems (pkgs: let
        package = self.packages.${pkgs.stdenv.hostPlatform.system}.solidcraft;
        desktop = { type = "app"; program = "${package}/bin/solidcraft"; };
      in {
        default = desktop;
        solidcraft = desktop;
        solvecraft = { type = "app"; program = "${package}/bin/solvecraft"; };
        cli = { type = "app"; program = "${package}/bin/solvecraft-cli"; };
      });
    };
}
