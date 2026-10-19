//! An Import's `file` can name a FreeCAD `.FCStd` or Rhino `.3dm` file (#1435, read through
//! cadmpeg); its `step` is still the STEP text the file was restated as, so the feature's fields
//! are unchanged and version 22 designs need no transformation; 0.13.x asks for an update before
//! it meets these imports.
