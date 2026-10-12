#!/usr/bin/env python3
"""Demo v2/v3: build an electronics enclosure in a running SolveCraft the way a person would,
with the parametric steps sent over MCP.

--story v3 (default): one standoff placed from the parameters and patterned 2 x 2 in the
Rectangular Pattern dialog, then width and wall changed in the Parameters dialog.
--story v2: four standoffs from one sketch, then the width changed over MCP.

    solvecraft --control 7878 &
    python3 examples/demo/enclosure_ui_mcp.py --port 7878 --cli solvecraft-cli [--story v2|v3]

Modelling steps go through the app's UI over its control channel: the pointer glides to a
toolbar button and clicks it (the real dialog opens), glides to the face or point it picks and
clicks, and values are typed into the dialog, as by hand. The port and boss sketches attach
directly to the enclosure's side face and inner floor, as in the original demo.
The camera follows like a user's would (Look At on a new sketch, back out to a 3/4 view on
Finish Sketch, a little orbit between features). The parameters and the dimensions that use them
are MCP `execute` calls through `solvecraft-cli mcp --connect` (enclosure_mcp.py's client).

--log writes one JSON line per caption and per pointer event, which record.sh turns into burnt-in
captions and a drawn cursor (the app gets synthetic pointer events, so the screen's own pointer
never moves).
"""

import argparse
import json
import math
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from enclosure_mcp import Mcp, control  # noqa: E402


# Seconds a caption stays up at least.
MIN_CAPTION = 1.4


class Demo:
    def __init__(self, port, mcp, log, speed, minimum_caption=MIN_CAPTION):
        self.port, self.mcp, self.log, self.speed = port, mcp, log, speed
        self.minimum_caption = minimum_caption
        self.pos = None
        self.stop = 10**6

    # -- logging -------------------------------------------------------------------------------
    def mark(self, **k):
        if self.log:
            self.log.write(json.dumps({"t": time.time(), **k}) + "\n")
            self.log.flush()

    def caption(self, text, via=""):
        self.n = getattr(self, "n", 0) + 1
        if self.n > self.stop:
            raise StopIteration
        # Every caption stays up long enough to read.
        shown = time.time() - getattr(self, "shown_at", 0.0)
        if shown < self.minimum_caption:
            time.sleep(self.minimum_caption - shown)
        self.shown_at = time.time()
        print(f"  {text}  [{via}]", file=sys.stderr)
        self.mark(caption=text, via=via)

    def wait(self, s):
        time.sleep(s * self.speed)

    # -- the app -------------------------------------------------------------------------------
    def ui(self, method, params=None):
        r = control(self.port, method, params)
        if not r.get("ok", False):
            raise RuntimeError(f"{method} {params}: {r.get('error')}")
        return r.get("result")

    def at(self, **where):
        # A clicked combo publishes its popup handles on the next rendered frame.
        # Observe that bounded transition; never substitute a command for the click.
        for attempt in range(50 if "handle" in where else 1):
            r = control(self.port, "ui.at", where)
            if r.get("ok", False):
                p = r["result"]
                return (p[0], p[1])
            if attempt + 1 < 50 and "handle" in where:
                time.sleep(0.1)
        raise RuntimeError(f"ui.at {where}: {r.get('error')}")

    def glide(self, to, dur=0.3):
        """Move the pointer to `to` along an eased path (the app sees every step: hover
        highlights follow it)."""
        frm = self.pos or (1150.0, 620.0)
        n = max(2, int(dur * 40))
        for i in range(1, n + 1):
            t = i / n
            e = t * t * (3 - 2 * t)
            x, y = frm[0] + (to[0] - frm[0]) * e, frm[1] + (to[1] - frm[1]) * e
            self.ui("ui.move", {"x": x, "y": y})
            self.mark(cursor=[x, y])
            time.sleep(dur / n)
        self.pos = to

    def click(self, to, dur=0.3, settle=0.2):
        self.glide(to, dur)
        self.ui("ui.click", {"x": to[0], "y": to[1]})
        self.mark(cursor=list(to), click=True)
        self.wait(settle)
        self.settle_camera()

    def tool(self, command, panel=None):
        """Click a toolbar button (opening its panel's drop-down first when it's in one)."""
        self.settle_camera()
        dialog = self.ui("ui.inspect").get("dialog") or ""
        if "error: Some(" in dialog:
            raise RuntimeError(f"the previous UI operation was refused: {dialog}")
        handles = self.ui("ui.handles")
        if panel and not any(h["name"] == f"toolbar:{command}" for h in handles):
            self.click(self.at(handle=panel), settle=0.3)
        self.click(self.at(handle=f"toolbar:{command}"), settle=0.35)

    def type(self, text, enter=True):
        self.ui("ui.text", {"text": text})
        self.wait(0.35)
        if enter:
            self.ui("ui.key", {"key": "Enter"})
            self.wait(0.5)
            self.settle_camera()

    def key(self, k):
        self.ui("ui.key", {"key": k})
        self.wait(0.15)
        self.settle_camera()

    def view(self, v, settle=0.7):
        if v == "home":
            # The shipping Axis View's Home button, with the same camera/style as v3.
            self.click(self.at(handle="axis_view:home"), settle=0.3)
        else:
            self.ui("ui.view", {"view": v, "animate": True})
        self.wait(settle)
        self.settle_camera()

        if v == "home":
            # Original Home also fitted the model. The current Axis View only
            # rotates: retaining the close sketch camera crops the first extrusion.
            self.ui("ui.view", {"view": "fit"})
            self.settle_camera()
            vp = self.ui("ui.inspect")["viewport"]
            self.ui("ui.scroll", {"x": vp[0] + vp[2] / 2, "y": vp[1] + vp[3] / 2, "delta": -250})
            self.settle_camera()

    def orbit(self, dx, dy=0, steps=40):
        """A right-button drag in the viewport, as a user orbits."""
        vp = self.ui("ui.inspect").get("viewport") or [250, 126, 1350, 800]
        cx, cy = vp[0] + vp[2] * 0.62, vp[1] + vp[3] * 0.55
        self.glide((cx, cy), 0.25)
        self.mark(drag_start=[cx, cy])
        self.ui("ui.drag", {"x0": cx, "y0": cy, "x1": cx + dx, "y1": cy + dy, "button": "right", "steps": steps})
        self.settle_camera()
        # The pointer moved with the camera (the app plays the drag one event a frame).
        self.mark(drag_end=[cx + dx, cy + dy])
        self.pos = (cx + dx, cy + dy)

    def drag(self, frm, to, button="left", steps=30):
        """Press at `frm`, move to `to` and let go (a window's title bar, or a middle-button pan)."""
        self.glide(frm, 0.25)
        self.mark(drag_start=list(frm))
        self.ui("ui.drag", {"x0": frm[0], "y0": frm[1], "x1": to[0], "y1": to[1], "button": button, "steps": steps})
        self.settle_camera()
        self.wait(0.2)
        self.mark(drag_end=list(to))
        self.pos = tuple(to)

    def settle_camera(self):
        """Wait for queued pointer events, moving dialogs and geometry previews."""
        last, still = None, 0
        for _ in range(1200):
            time.sleep(0.1)
            state = self.ui("ui.inspect")
            current = json.dumps([state.get("camera"), state.get("tool"), self.ui("ui.handles")], sort_keys=True)
            busy = state.get("preview", {}).get("busy", False)
            still = still + 1 if current == last and not busy else 0
            last = current
            if still >= 3:
                return
        raise RuntimeError("the camera/dialog/preview did not settle within 120 seconds")

    def execute(self, command, params):
        self.settle_camera()
        self.mcp.tool("execute", {"command": command, "params": params})
        self.settle_camera()


def datum_sketch(d, base, offset):
    """Create a parametric datum in the real dialog, then select its browser row."""
    d.ui("ui.set", {"showOrigin": True})
    d.tool("construct.plane.offset", panel="panel:SOLID:CONSTRUCT")
    d.click(d.at(plane=base), settle=0.3)
    d.type(offset)
    names = [h["name"] for h in d.ui("ui.handles") if h["name"].startswith("timeline:")]
    plane = names[-1].split(":", 1)[1]
    d.datums = getattr(d, "datums", []) + [plane]
    d.tool("sketch.create")
    d.click(d.at(handle=f"row:{plane}"), settle=0.8)
    d.settle_camera()
    d.ui("ui.set", {"showOrigin": False})


def build_box(d, params):
    d.caption("Parameters over MCP: " + ", ".join(f"{n} {e.split()[0]}" for n, e in params), "MCP tools/call execute parameters.change")
    for name, expr in params:
        d.execute("parameters.change", {"name": name, "expression": expr})
        d.wait(0.25)
    d.wait(0.6)

    d.caption("Create Sketch on the XY plane", "UI click")
    d.tool("sketch.create")
    d.click(d.at(plane="XY"), settle=0.8)
    d.settle_camera()

    d.caption("Center rectangle from the origin", "UI click")
    d.tool("sketch.rectangle.center")
    d.click(d.at(sketch=[0, 0]), settle=0.15)
    d.click(d.at(sketch=[40, 30]), dur=0.45, settle=0.2)
    d.key("Escape")

    d.caption("Constraints over MCP: centred, width × depth", "MCP tools/call execute sketch.dimension")
    sk = json.loads(d.mcp.tool("execute", {"command": "sketch.inspect", "params": {}}))
    sk = sk.get("result", sk)
    centre = next((p["id"] for p in sk.get("points", []) if p["id"] != "origin" and abs(p["at"][0]) + abs(p["at"][1]) < 1e-6), None)
    if centre:
        d.execute("sketch.constraint.coincident", {"a": centre, "b": "origin"})
    d.execute("sketch.dimension", {"entities": ["l1"], "value": "width"})
    d.wait(0.3)
    d.execute("sketch.dimension", {"entities": ["l2"], "value": "depth"})
    d.wait(0.8)

    d.caption("Finish Sketch", "UI click")
    d.tool("sketch.finish")
    # The origin planes are in the way of picking faces from here on.
    d.ui("ui.set", {"showOrigin": False})
    d.view("home", 0.6)

    d.caption("Extrude: height", "UI click")
    # The first Fit framed a flat sketch at z=0. Leave room for the upcoming
    # 30 mm height instead of letting its far corner grow above the viewport.
    vp = d.ui("ui.inspect")["viewport"]
    d.ui("ui.scroll", {"x": vp[0] + vp[2] / 2, "y": vp[1] + vp[3] / 2, "delta": -250})
    d.settle_camera()
    d.tool("solid.extrude")
    d.wait(0.3)
    d.type("height")

    d.caption("Fillet the four vertical edges: 6 mm", "UI click")
    d.tool("solid.fillet")
    for p in ([40, -30, 15], [-40, -30, 15]):
        d.click(d.at(world=p), settle=0.15)
    d.orbit(-260, 20)
    for p in ([40, 30, 15], [-40, 30, 15]):
        d.click(d.at(world=p), settle=0.2)
    d.type("6")
    d.view("home", 0.5)

    d.caption("Shell: open the top face, wall thickness", "UI click")
    d.tool("solid.shell", panel="panel:SOLID:MODIFY")
    d.click(d.at(world=[0, 0, 30]), settle=0.3)
    d.type("wall")

    d.caption("Sketch on the side face for a USB port", "UI click")
    d.tool("sketch.create")
    d.click(d.at(world=[40, 10, 20]), settle=0.9)
    d.settle_camera()
    d.tool("sketch.rectangle.center")
    d.click(d.at(world=[40, 0, 12]), settle=0.15)
    d.click(d.at(world=[40, 6, 15.5]), dur=0.4, settle=0.2)
    d.key("Escape")
    d.tool("sketch.finish")
    d.view("home", 0.5)

    # Over MCP with the profile named by a point inside it: a profile picked in the Extrude
    # dialog is kept by its index, which a face sketch renumbers when the face changes shape.
    d.caption("Cut the port through the wall", "MCP tools/call execute solid.extrude")
    d.execute("solid.extrude", {"profiles": [{"point": [0, 12]}], "distance": "-wall", "operation": "cut"})
    d.wait(0.6)
    d.orbit(-140, 30)

def run_v2(d):
    """v2: four standoffs from one sketch, then the width changed over MCP."""
    build_box(d, [("width", "80 mm"), ("depth", "60 mm"), ("height", "30 mm"), ("wall", "2 mm")])
    d.caption("Sketch on the inner floor: four standoffs", "UI click")
    d.tool("sketch.create")
    d.click(d.at(world=[6, -9, 2]), settle=0.9)
    d.settle_camera()
    d.tool("sketch.circle.center")
    for x, y in [(-30, -20), (30, -20), (30, 20), (-30, 20)]:
        d.click(d.at(world=[x, y, 2]), dur=0.3, settle=0.1)
        d.click(d.at(world=[x + 3.5, y, 2]), dur=0.2, settle=0.15)
    d.key("Escape")
    d.tool("sketch.finish")

    d.view("home", 0.5)

    d.caption("Extrude the standoffs: 12 mm", "MCP tools/call execute solid.extrude")
    d.execute("solid.extrude", {"profiles": [{"point": [x, y]} for x, y in [(-30, -20), (30, -20), (30, 20), (-30, 20)]], "distance": 12, "operation": "join"})
    d.wait(0.4)

    d.caption("Change one parameter: width 80 → 100 mm", "MCP tools/call execute parameters.change")
    d.execute("parameters.change", {"name": "width", "expression": "100 mm"})
    d.wait(1.8)

    d.caption("", "")
    d.orbit(420, 0, steps=90)
    d.view("home", 1.5)


def select_all_and_type(d, at, text, enter=True):
    """Click a value field, select what is there (Ctrl+A) and type over it."""
    d.click(at, settle=0.15)
    d.ui("ui.key", {"key": "A", "cmd": True})
    d.wait(0.1)
    d.type(text, enter)


def run_v3(d):
    """v3: one standoff placed from the parameters and patterned 2 x 2 in the Rectangular Pattern
    dialog, then width and wall changed in the Parameters dialog."""
    build_box(d, [("width", "80 mm"), ("depth", "60 mm"), ("height", "30 mm"), ("wall", "2 mm"), ("inset", "10 mm")])

    d.caption("Sketch one standoff on the inner floor", "UI click")
    d.tool("sketch.create")
    d.click(d.at(world=[6, -9, 2]), settle=0.9)
    d.settle_camera()
    d.tool("sketch.circle.center")
    d.click(d.at(world=[-30, 20, 2]), dur=0.3, settle=0.1)
    d.click(d.at(world=[-26.5, 20, 2]), dur=0.2, settle=0.15)
    d.key("Escape")

    d.caption("Place it from the parameters: width/2 − inset, depth/2 − inset", "MCP tools/call execute sketch.dimension")
    sk = json.loads(d.mcp.tool("execute", {"command": "sketch.inspect", "params": {}}))
    sk = sk.get("result", sk)
    circles = [c["id"] for c in sk["curves"] if c["type"] == "circle" and not c.get("fixed")]
    if not circles:
        raise RuntimeError(f"no circle in the standoff sketch: {[c['id'] for c in sk['curves']]}")
    circle = circles[0]
    d.execute("sketch.dimension", {"entities": ["origin", f"{circle}.center"], "type": "horizontal", "value": "width / 2 - inset"})
    d.execute("sketch.dimension", {"entities": ["origin", f"{circle}.center"], "type": "vertical", "value": "depth / 2 - inset"})
    d.execute("sketch.dimension", {"entities": [circle], "type": "diameter", "value": "7 mm"})
    d.wait(0.8)
    d.tool("sketch.finish")
    d.view("home", 0.5)

    d.caption("Extrude the standoff: 12 mm", "MCP tools/call execute solid.extrude")
    d.execute("solid.extrude", {"profiles": [[circle]], "distance": 12, "operation": "join"})
    d.wait(0.7)
    standoff = d.ui("ui.handles", {"prefix": "timeline:"})[-1]["name"]

    d.caption("Rectangular Pattern: 2 × 2, spaced width − 2·inset by depth − 2·inset", "UI click")
    d.tool("solid.pattern.rectangular", panel="panel:SOLID:CREATE")
    d.click(d.at(handle="dialog:Object Type"), settle=0.25)
    d.click(d.at(handle="dialog:Object Type:Features"), settle=0.25)
    d.click(d.at(handle=standoff), settle=0.25)
    d.click(d.at(handle="dialog:Direction"), settle=0.15)
    d.click(d.at(world=[0, 30, 30]), settle=0.2)
    d.click(d.at(handle="dialog:Direction 2"), settle=0.15)
    d.click(d.at(world=[-40, 0, 30]), settle=0.2)
    select_all_and_type(d, d.at(handle="dialog:Quantity"), "2", enter=False)
    select_all_and_type(d, d.at(handle="dialog:Spacing"), "width - 2*inset", enter=False)
    select_all_and_type(d, d.at(handle="dialog:Quantity 2"), "2", enter=False)
    select_all_and_type(d, d.at(handle="dialog:Spacing 2"), "-(depth - 2*inset)", enter=False)
    d.wait(0.4)
    d.click(d.at(handle="dialog:OK"), settle=0.4)
    # The original small orbit reveals the corners behind the walls.
    d.orbit(-60, 45)
    # Raise the view enough to see all four 12 mm bosses behind the 30 mm walls.
    d.orbit(0, 35)

    d.key("Escape")

    d.caption("Parameters dialog: width 80 → 100 mm", "UI click")
    d.tool("parameters.change", panel="panel:SOLID:MODIFY")
    d.wait(0.3)
    # Original framing: move the dialog aside and pan the model beside it.
    title = d.at(handle="params:title")
    d.drag(title, (title[0] - 330, title[1] + 20), steps=25)
    vp = d.ui("ui.inspect").get("viewport") or [250, 126, 1350, 800]
    d.drag((vp[0] + vp[2] * 0.62, vp[1] + vp[3] * 0.5), (vp[0] + vp[2] * 0.62 + 280, vp[1] + vp[3] * 0.5), button="middle", steps=25)
    select_all_and_type(d, d.at(handle="param:width"), "100 mm")
    d.wait(1.0)
    d.caption("… and wall 2 → 3 mm", "UI click")
    select_all_and_type(d, d.at(handle="param:wall"), "3 mm")
    d.wait(0.8)
    d.click(d.at(handle="params:close"), settle=0.4)

    d.caption("", "")
    # Keep the original demo's lower three-quarter silhouette.
    d.view("home", 0.5)
    d.wait(1.5)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--port", type=int, default=7878, help="the app's --control port")
    ap.add_argument("--cli", default="solvecraft-cli", help="path to solvecraft-cli")
    ap.add_argument("--log", help="write a JSON line per caption and pointer event")
    ap.add_argument("--story", choices=["v2", "v3"], default="v3", help="v2: standoffs from one sketch, width over MCP; v3: one standoff patterned, Parameters dialog")
    ap.add_argument("--until", type=int, default=10**6, help="stop before step N (trying the script out)")
    ap.add_argument("--speed", type=float, default=0.55, help="scale the pauses (2 = twice as long)")
    ap.add_argument("--save", help="save the completed healthy enclosure and measured proof")
    ap.add_argument("--minimum-caption", type=float, default=MIN_CAPTION, help="minimum real-time caption duration before playback acceleration")
    a = ap.parse_args()
    if not math.isfinite(a.speed) or a.speed <= 0 or not math.isfinite(a.minimum_caption) or a.minimum_caption < 0:
        ap.error("pause scale must be positive and caption duration nonnegative")
    mcp = Mcp([a.cli, "mcp", "--connect", f"127.0.0.1:{a.port}"])
    mcp.send("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "enclosure-demo-v2", "version": "2"}})
    mcp.send("notifications/initialized", notify=True)
    d = Demo(a.port, mcp, open(a.log, "w") if a.log else None, a.speed, a.minimum_caption)
    d.ui("ui.prefs", {"language": "en", "theme": "dark", "ui_scale": 1, "nav": "fusion", "button": "apply"})
    d.ui("ui.resize", {"width": 1920, "height": 1080})
    d.ui("ui.home", {"open": False})
    d.ui("ui.documents", {"action": "new"})
    d.ui("ui.home", {"open": False})
    d.ui("ui.set", {"showOrigin": True})
    d.settle_camera()
    d.stop = a.until - 1
    t0 = time.time()
    try:
        run_v3(d) if a.story == "v3" else run_v2(d)
    except StopIteration:
        pass
    if a.save and a.until == 10**6:
        # Keep this health/save/reopen proof in the raw take, outside public playback.
        d.mark(private_start=True)
        d.settle_camera()
        design = d.ui("document.inspect", {"measure": True})
        if len(design.get("bodies", [])) != 1 or any(f.get("error") or f.get("warning") for f in design.get("timeline", [])):
            raise RuntimeError(f"the completed enclosure is unhealthy: {design}")
        expected_width = 100
        bbox = design["bodies"][0]["bbox"]
        if any(abs(v - want) > 1e-5 for side, expected in [("min", [-expected_width / 2, -30, 0]), ("max", [expected_width / 2, 30, 30])] for v, want in zip(bbox[side], expected)):
            raise RuntimeError(f"the enclosure did not rebuild to 100 x 60 x 30 mm: {design}")
        values = {p["name"]: p.get("value") for p in design.get("params", []) if not p.get("error")}
        for name, expected in [("width", 100), ("wall", 3 if a.story == "v3" else 2)]:
            if values.get(name) != expected:
                raise RuntimeError(f"the final {name} parameter is not {expected}: {design}")
        d.execute("file.save_as", {"path": a.save})
        saved = json.load(open(a.save))
        fillets = [f for f in saved["features"] if f.get("type") == "fillet"]
        if len(fillets) != 1 or len(fillets[0].get("edges", [])) != 4:
            raise RuntimeError("the enclosure did not retain all four selected corner fillets")
        with open(a.save + ".measure.json", "w") as proof:
            json.dump(design, proof, indent=2)
        # Reopen the saved design for the ending: the live pattern preview can leave
        # stale display edges behind. This is the actual app, not a rendered substitute.
        d.ui("ui.documents", {"action": "close", "force": True})
        d.ui("ui.file", {"path": a.save})
        for _ in range(1200):
            if not d.ui("ui.inspect").get("file_busy", False):
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("the saved enclosure did not reopen")
        d.ui("ui.home", {"open": False})
        d.settle_camera()
        d.ui("ui.set", {"showOrigin": False, "showSketches": False})
        d.mark(private_end=True)
        d.ui("ui.view", {"view": "home"})
        d.settle_camera()
        d.orbit(0, 80)
        d.ui("ui.view", {"view": "fit"})
        d.settle_camera()
        vp = d.ui("ui.inspect")["viewport"]
        d.ui("ui.scroll", {"x": vp[0] + vp[2] / 2, "y": vp[1] + vp[3] / 2, "delta": -120})
        d.settle_camera()
        d.glide((1500, 730), 0.4)
    d.mark(caption="", via="")
    mcp.close()
    print(f"done in {time.time() - t0:.1f}s", file=sys.stderr)


if __name__ == "__main__":
    main()
