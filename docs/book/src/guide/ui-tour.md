# A tour of the window

![The SolidCraft window](../generated/sample-plate.png)

- **Title bar.** On Windows and Linux the application bar is the window's title bar: File, save,
  undo and redo, the start page, the theme, Help, and one tab per open design (each with its own
  undo history, selection and camera).
- **Toolbar.** The workspace switcher (DESIGN) sits on the left. The tabs (SOLID, SURFACE,
  SHEET METAL, PLASTIC, UTILITIES, and SKETCH while you sketch) hold panels of commands. Click a
  panel's name for all its commands.
- **Browser** (left). The design as a tree: document settings and units, named views, the
  origin, bodies, sketches, construction planes, canvases, joints and components. The eye shows or
  hides an item, a component's radio button makes it the active component, and right-clicking any
  row gives that item's menu. Rows drag into groups and other components.
- **Viewport.** Click to select, Shift or Ctrl to add, drag for a window or crossing box. The
  wheel zooms at the cursor, the middle button pans and right-drag orbits freely (the mouse
  preset in Preferences switches to SolidWorks or Inventor style). Shift+middle orbits freely
  in the default and Inventor presets. Shift+right-drag snaps to the nearest standard or
  diagonal view; Shift does not request snapping when the preset already requires it to orbit.
  The axis view shows the world's X/Y/Z directions in SolidCraft's colors. Click a signed
  axis tip to look along that axis; click the aligned tip again to look from the opposite side.
  Drag the axis view to orbit around the view centre, with Shift for snapping. Top and bottom
  keep the nearest quarter-turn heading. The house and navigation bar's Home button go home.
  Right-click for the marking menu: eight commands around the cursor, plus a
  menu for whatever is under it.
  Shaded views omit certified smooth joins between spline spans on extruded
  walls. Real creases, cap outlines and authored face splits remain visible.
  Wireframe retains its edge presentation, and hiding a join does not remove its CAD
  identity or change the exported solid. Unsupported or uncertain wall geometry
  keeps its existing edge presentation.
- **Dialogs** dock on the right with a live preview. Most features also show drag handles and a
  value box in the viewport.
- **Timeline** (bottom). Every feature in order. Double-click to edit, drag the marker to roll
  back, and drag features to reorder them.
- **S box.** Press S anywhere for a command search at the cursor. Pin the commands you use most
  to its top.

Preferences (File › Preferences) hold the theme, the units of new designs, the mouse preset, the
visual style and autosave. Help › Keyboard Shortcuts lists every key, and File › Keyboard
Shortcuts changes them.

Navigation › **Smooth wheel zoom** is on by default. Turn it off to apply each wheel or
trackpad event immediately, with no smoothing tail. It is independent of **Animate view
changes**, which controls axis views, Look At, Home and Fit. Re-enabling smoothing during
a wheel gesture takes effect after that gesture ends, avoiding leftover zoom being replayed.

Shift-orbit snapping includes 45-degree edge views and isometric corner views, with a
lightly damped spring transition and a small boundary margin to prevent flicker.
