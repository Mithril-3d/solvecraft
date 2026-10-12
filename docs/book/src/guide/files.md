# Import and export

| Format | Read | Write |
|---|---|---|
| `.solvecraft` (JSON design, versioned) | yes | yes |
| STEP (AP203, AP214, AP242) | solids, assemblies, units, names, colours | AP242 with names, colours and the component tree |
| IGES 5.3 | manifold solids, or trimmed surfaces sewn into solids | manifold solids with names and colours |
| 3MF, STL, OBJ | mesh bodies | yes |
| DXF, SVG | into a sketch | sketches and flat patterns as DXF |
| PNG, JPEG | canvases and decals | |

Opening a STEP or IGES file creates a design with the file's bodies as a base feature, and
inserting one adds them to the current design. Later features build on them. Mesh bodies render,
measure, move and export, but solid features need B-rep bodies.

Saves are atomic: a crash at any moment leaves either the old file or the new one, and the
replaced version is kept as `<file>.bak`. Unsaved changes are autosaved to a recovery folder,
and the next launch offers to recover them. Older design files are upgraded on open, and files
from a newer version are refused with a message.

Current designs use format 12, including persistent analytic offset boundary roles and
association profile references, durable sketch face anchors, periodic fit splines and versioned
revolve conventions. It also retains recorded body transfers, explicit 3D curve coordinate
spaces and certified face bindings, alongside semantic configuration styles, associative
free-form offsets and cylindrical emboss selections. Older ownership maps, geometry and
expressions retain their saved meaning. Legacy 3D curves with retained associations receive
their producer's coordinate-space marker without changing any saved points; detached curves
retain world coordinates. A lost cross-component association refreshed by an older build may
need an explicit relink or refresh when its source returns. Loading never guesses a numerical
correction. Association records, coordinate arrays and placements are bounded and checked.

## Saving in the web app

In the web app, **Save** and **Save As** keep designs in this browser's private storage;
they do not open a dialog for choosing a folder on your computer. Clearing the site's data
can remove those designs. Download a copy to keep it outside the browser or move it to
another device.

1. Open the File menu and choose **Open** to show **Files in this browser**.
2. Choose **Download open design** for the current design, including its unsaved changes,
   or **Download** beside a stored file for its last saved contents.
3. Find the `.solvecraft` file in your browser's downloads. Whether a folder chooser appears
   depends on your browser's download settings.

Use **Open from computer…** in the same panel to reopen a downloaded design. The web app
works on a browser-stored copy; later saves do not update the original downloaded file.
Use **Rename** beside a stored file to change its name. Exports such as STEP and STL are
also downloaded.
