# Writes block.FCStd: a 10 x 20 x 30 mm box called "Block". Run inside FreeCAD as an external
# program, from this directory: `freecadcmd make_block_fcstd.py` (made with FreeCAD 1.1.4).
import os

import FreeCAD
import Part  # noqa: F401  (loads the Part workbench)

doc = FreeCAD.newDocument("Block")
box = doc.addObject("Part::Box", "Block")
box.Length, box.Width, box.Height = 10, 20, 30
doc.recompute()
doc.saveAs(os.path.join(os.getcwd(), "block.FCStd"))
