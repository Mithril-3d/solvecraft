# Contributing to SolidCraft

SolidCraft is an open-source parametric 3D CAD app written in pure Rust.
You sketch on a plane, constrain the sketch, turn profiles into solids, and
change a parameter to watch the timeline rebuild. It runs as a desktop app (Windows, Linux), in
the browser (wasm + WebGPU/WebGL2), headless from a CLI, and as an MCP server that AI agents can
model with.

Help is welcome, whether that's a one-line fix or owning a whole area. This page covers how to
build it, how the code is laid out, the few rules that aren't negotiable, and where help is
needed most.

## Get it running (about 10 minutes on a fast machine)

You need stable Rust 1.95 or newer (required by egui/eframe 0.36). For the web
build you also need `rustup target add wasm32-unknown-unknown` and [`trunk`](https://trunkrs.dev).

```sh
git clone https://github.com/bherbruck/solidcraft && cd solidcraft
cargo run --release -p solvecraft -- --sample      # desktop app with a sample part
cargo xtask test                                   # unit, engine and UI-scenario tests
cargo xtask ci                                     # everything a PR must pass (see below)
```

SolidCraft is the product and repository name. Cargo packages and executables retain
their `solvecraft` names for compatibility with existing scripts and MCP clients.
Build the desktop app with `cargo build --release -p solvecraft`, or the CLI with
`cargo build --release -p solvecraft-cli`. Run the CLI with
`cargo run --release -p solvecraft-cli -- commands` (or `target/release/solvecraft-cli commands`
after building). There is no Cargo package named `solidcraft`.

Web: `cd apps/solvecraft-web && trunk serve --release`, then open the printed URL (`?sample`
opens the sample part, `?webgl` forces WebGL2). Saving and opening files in the browser needs
HTTPS or localhost, because it uses the Origin Private File System.

Windows from Linux: `cargo xwin build --release --target x86_64-pc-windows-msvc -p solvecraft`.

### NixOS: `nix develop`

NixOS has no `/usr/lib`, so nothing is on `LD_LIBRARY_PATH` and a plain `cargo run` finds none of
the libraries the app loads at run time: the Wayland ones winit opens the window through, the EGL
loader wgpu renders through, and libdbus-1, which rfd's file dialogs reach xdg-desktop-portal
through (a dialog then never appears and the pick returns no path):

```
ERROR solvecraft: winit EventLoopError: ... The wayland library could not be loaded
ERROR eframe::native::run: Exiting because of error: WGPU error: Failed to create surface for any enabled backend: {}
ERROR rfd::backend::xdg_desktop_portal::portal::libdbus: Can't connect to a portal: libdbus-1.so not found
```

`nix develop` (the [`flake.nix`](flake.nix) in the repository root) supplies Wayland,
X11, EGL, Vulkan-loader and D-Bus runtime libraries. It also exposes the NixOS
driver manifests in `/run/opengl-driver/share`, so Vulkan can discover the system
drivers. Enable `hardware.graphics.enable` on NixOS; other Linux distributions
may need a matching [nixGL wrapper](https://github.com/nix-community/nixGL).
The default development shell retains the Rust/C toolchain already on `PATH`;
`nix develop .#rust` supplies the pinned toolchain and builds in `target/nix`.

To launch, use `nix run .#solidcraft -- --sample`. To install the app's terminal
commands, desktop entry and generated icons, use `nix profile install .#solidcraft`.
`nix build` alone creates a build result; it does not install a launcher in your
desktop menu. The installed `solidcraft` command and compatible `solvecraft`
command run the same wrapped app; `solvecraft-cli` is included as well. See
[Nix installation and troubleshooting](docs/nix.md) for remote-flake commands,
desktop integration and how to distinguish an old executable on `PATH`.

## How the code is laid out

The [README](README.md#architecture) has the layer table. The short version:

- `crates/geom`: vectors, planes, meshes, measures.
- `crates/sketch`: our own constraint solver, plus profile finding.
- `crates/kernel`: the B-rep kernel boundary. It wraps [truck](https://github.com/ricosjp/truck),
  and it's the only crate allowed to touch truck. Fillets, chamfers, shell/offset, Delete Face,
  Split Face, STEP/IGES read and write, and face provenance are our own code here. Patched
  copies of three truck crates live in `vendor/` and are excluded from our fmt/clippy rules.
- `crates/doc`: the document. It holds parameters and expressions, the feature timeline and its
  incremental rebuild, components and occurrences, joints, appearances, and persistent naming.
- `crates/io`: design files and mesh formats (3MF, STL, OBJ).
- `crates/engine`: the session and the command registry. **Everything the user can do is a
  command**, and the toolbar, dialogs, scripts, CLI, control channel and MCP all run the same
  commands.
- `crates/ui-egui`: the desktop/web front end (egui + wgpu). It's swappable; nothing below it
  depends on egui.
- `crates/mcp`, `apps/solvecraft`, `apps/solvecraft-cli`, `apps/solvecraft-web`.

`cargo xtask layers` fails the build if a lower layer depends on a higher one.

## Rules that aren't negotiable

1. **Clean-room.** We observe Fusion as a black box: its UI, how parts it builds measure, and
   what it does with a given input. We never copy Autodesk code, icons, artwork, help text,
   tooltips, templates or materials, and we never commit files Fusion produced. Command ids are
   our own (`solid.fillet`, not Fusion's internal names).
2. **No GPL/LGPL/AGPL code, not even reading it.** That rules out OpenCASCADE, FreeCAD
   (including planegcs), SolveSpace, CadQuery/OCP, LibreCAD and QCAD. SolidCraft is
   MIT OR Apache-2.0, and its solver and blends are written from the maths. Permissive
   dependencies are fine. Note which one you're adding and why in the PR, and run
   `cargo xtask licences` so THIRD-PARTY-LICENSES.txt lists it (CI checks that it is current).
3. **Every asset is original or openly licensed and has a row in
   [ATTRIBUTION.md](ATTRIBUTION.md).** Icons are drawn in code (`crates/ui-egui/src/icons.rs`).
   `cargo xtask assets` enforces this.
4. **Never crash.** People trust a CAD app with their work.
   - Non-test code never uses `unwrap`, `expect`, `panic!` or indexing on input-derived data.
     There's no `unsafe`, and clippy denies it.
   - An unsupported case returns an error ("not supported yet: …"); it never panics.
   - Every call into truck goes through `solvecraft_kernel::guard`.
   - Every crash fix comes with a regression test.
5. **Everything is a command, with a test.** A new feature means a `CommandSpec` in
   `crates/engine/src/cmd/`, engine tests that measure the result (volume, area,
   face/edge/vertex counts against a value you can derive), and for UI work a scenario (below).

[AGENTS.md](AGENTS.md) has the long form of these rules. It's written for AI agents, but it's
the most complete description of the house style.

## Tests

| What | Where | Run |
|---|---|---|
| Unit and engine tests | next to the code, or `*_tests.rs` files | `cargo xtask test` (or `cargo test --workspace --profile ci`) |
| UI scenarios | `crates/ui-egui/tests/scenarios/*.json` | part of `cargo test` |
| Fusion oracle | `cargo xtask oracle` | needs the local oracle data (see below) |

**UI scenarios** drive the real app headless: start a tool, click a plane or a sketch point,
press keys, then assert on the result.

```json
[
  {"start": "sketch.create"},
  {"click": {"plane": "XY"}},
  {"start": "sketch.rectangle.two_point"},
  {"click": {"sketch": [0, 0]}},
  {"click": {"sketch": [40, 30]}},
  {"expect": {"errors": 0}}
]
```

A new `.json` file in that folder is picked up automatically. To record a bug before it's fixed,
put `{"pending": "why"}` in the scenario, and remove it in the PR that fixes the bug.

**Use the `ci` profile.** CI runs the tests with `--profile ci`: light optimisation, line tables,
no LTO, and debug assertions off (truck has debug-only assertions that reject valid geometry). It
builds several times faster than `--release`. If a test fails only in a plain debug build, say so
in the PR. `cargo xtask test` builds the tests with it and then runs every test binary at once
(`cargo test` runs them one after another); arguments after `--` go to each binary, e.g.
`cargo xtask test -- shell`. Tests run beside each other on a busy machine, so a test must not
depend on wall-clock timing.

**The Fusion oracle** is a set of 76 reference parts, each built in Fusion from a recipe.
SolidCraft rebuilds the same recipe and compares volume, area and topology counts with Fusion's
measurements; [docs/oracle.md](docs/oracle.md) has the current results. The recipes and
measurements live outside the repo (`plan/`, gitignored), because they come from a Fusion
install. If you work on geometry and want to check against it, ask in an issue.

## Sending a change

1. Work is tracked in [GitHub issues](https://github.com/bherbruck/solidcraft/issues), labelled
   by area (`area:kernel`, `area:sketch`, `area:doc`, `area:ui`, `area:infra`). Before starting,
   comment on the issue so it gets the `in progress` label and two people don't build the same
   thing. For anything bigger than a bug fix that has no issue yet, open one first.
2. Branch from `main` and keep the PR to one topic.
3. Run `cargo xtask ci` before pushing. It runs fmt, the asset, licence and layer checks,
   clippy (`-D warnings`), then the tests with the wasm build check alongside. A PR has to pass it.
4. Reference the issue (`Fixes #12`). In the PR, say what you tested and how. For geometry work, include the measured numbers (and
   the expected ones).

Help reviewers see what the words mean. For observable CAD, UI or model behavior, please
include actual descriptive screenshots or a short recording in the issue or PR. This evidence
is required for changes or bugs that can be demonstrated in the app, including kernel,
geometry, assembly and backend work. Coding agents should run a small reproduction and
capture the evidence themselves; please don't ask a human for evidence the agent can produce.
Show the actual failure or result, preferably before and after, with brief steps and
expected/actual behavior. An arrow or annotation can help identify the detail that matters.

Flowcharts and Mermaid diagrams cannot replace evidence of visible behavior. Diagrams are
useful for architectural reasoning when actual screenshots cannot illustrate the point.
Changes with no useful observable behavior do not need screenshot or video evidence. Use a
small example without private designs or sensitive details; attach the media or link to an
accessible recording rather than committing generated review artifacts.

Keep checking upstream commits and the relevant PR's merge status throughout the work and
before handoff. Integrate upstream merges, resolve conflicts without losing others' changes,
and validate the integrated result. Before pushing, run `git pull --rebase origin main`, re-run
CI if it brought changes, and push without force. Report the upstream/PR state you checked,
any conflicts resolved and validation results. While a task remains active, follow an open
PR through its status changes; at handoff, clearly identify anything still awaiting merge.

Much of SolidCraft so far was written by AI coding agents (Claude) working in parallel, each
owning one area, with a human setting direction and reporting bugs. Their working notes are in
[AGENTS.md](AGENTS.md). Human contributors are just as welcome. Use whatever tools you like, as
long as the change meets the rules above.

## Where help is wanted most

See the [`help wanted`](https://github.com/bherbruck/solidcraft/labels/help%20wanted) and
[`good first issue`](https://github.com/bherbruck/solidcraft/labels/good%20first%20issue) labels.
In order of how much they'd help users:

1. **Fillet and blend robustness.** Fillets are our own code (truck has no fillet operation).
   Constant, variable-radius and chord fillets work, and so do corners where fillets of
   different radii meet, including three rounds meeting at a vertex. Next are setback corners, fillets along curved edges between curved
   faces, and fillets that consume neighbouring faces.
2. **Boolean robustness.** A random-design fuzzer currently gets about half of its designs
   through without a kernel error. Each failing seed is a reproducible bug.
3. **Y-up / Z-up.** SolidCraft is Z-up only, so most STEP files (from SolidWorks, Onshape, or
   Fusion's default) open lying on their backs. Fusion solves this with a "default modeling
   orientation" preference.
4. **Imported appearances.** Colours from STEP files are lost when a feature changes the
   imported body. They should become design appearances on import.
5. **Drawings, CAM and mesh tools.** These are out of scope for now (see
   [ROADMAP.md](ROADMAP.md)), but anyone who wants to own one is welcome.

## Licence

By contributing you agree that your contribution is licensed as MIT OR Apache-2.0, like the rest
of SolidCraft. SolidCraft isn't affiliated with Autodesk. Fusion is a trademark of Autodesk, Inc.
