# Components and joints

A design is a tree of **components**. **New Component** (`component.create`) adds one, and
**Create Components from Bodies** (`component.from_bodies`) turns bodies into components. Each
component has its own origin, bodies, sketches and timeline entries. The **active component**
(the radio button in the browser) receives new sketches and features; the others are drawn
faded.

A component is placed by its **occurrences**. Paste creates another instance of the same
component, and **Paste New** creates an independent copy. Move an occurrence with the move
triad, then **Capture Position** to keep it or **Revert** to put it back. SolidCraft asks before
the next command if a move wasn't captured. **Ground** fixes an occurrence in place.

Paste New copies nested component definitions, sketches, feature histories and generated
dimension parameters. Repeated child instances remain shared within the new copy. Materials,
appearances and wholly internal joints/motion records receive copied references. Design-wide
user parameters remain shared; retained external links and omitted cross-boundary motion
records are reported. Unsupported destructive links refuse the entire copy. See
[individual-instance and copy details](https://github.com/bherbruck/solidcraft/blob/main/docs/occurrence-workflows.md).

**Joints** (`joint.create`, J) connect two components by joint origins that snap to faces, edges,
circles and vertices: rigid, revolute, slider, cylindrical, pin-slot, planar and ball, with
offsets, flip and limits. **As-built Joint** keeps the current position, **Rigid Group** locks
components together, **Drive Joints** moves a joint through its values, and **Motion Link**
couples two joints. Contact sets stop motion where bodies would pass through each other,
**Motion Study** keys joint values along a timeline, and **Interference** shows overlapping
volumes in red.

STEP export writes the component tree as an assembly.

Right-click a component in the Browser and choose **Properties** to inspect
material-aware mass, volume, surface area, centre of mass and world bounds. Descendants and
repeated instances contribute to assembly totals; pending moves are included. A component
uses its first placement; `occurrence.properties` selects a specific instance. Root Properties
measures the complete assembly. Volumes are summed, including overlapping bodies, rather than
measuring their union. The same workflow is available through `component.properties` and
`occurrence.properties`; see
[placement and measurement details](https://github.com/bherbruck/solidcraft/blob/main/docs/assembly-properties.md).
