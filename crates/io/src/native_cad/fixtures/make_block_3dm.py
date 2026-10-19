# Writes block.3dm: a 10 x 20 x 30 mm box called "Block", as a Rhino 7 file, with McNeel's MIT
# licensed rhino3dm package. Run from this directory: `uv run --with rhino3dm python make_block_3dm.py`.
import rhino3dm

model = rhino3dm.File3dm()
model.Settings.ModelUnitSystem = rhino3dm.UnitSystem.Millimeters
brep = rhino3dm.Brep.CreateFromBox(rhino3dm.Box(rhino3dm.BoundingBox(0, 0, 0, 10, 20, 30)))
attributes = rhino3dm.ObjectAttributes()
attributes.Name = "Block"
model.Objects.AddBrep(brep, attributes)
model.Write("block.3dm", 7)
