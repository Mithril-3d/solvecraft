# Contact sets and joint motion

Contact sets constrain **Drive Joints**, motion-study playback and exported motion samples.
Bodies belonging to different members of an enabled set are checked along the requested
joint motion. A collision between clear start and end poses can therefore stop the motion,
including slider travel through a blocker and revolute travel across one.

```json
{"command":"contact.create","params":{"occurrences":["Base:1","Carriage:1"]}}
{"command":"joint.drive","params":{"joint":"Rail","value":30}}
```

A stopped command returns `stopped_by_contact`, identifying the occurrence pair and the
fraction of the requested motion reached. The first detected collision interval is refined
to leave the bodies just short of penetration, within the tessellated contact tolerance.
Motion links and limits participate in the trajectory checks. Moving a body between
components preserves its placement for contact checks through its body offset.

A drive can also stop where a closed loop of joints (a four-bar, a slider-crank) can no longer
close; it then returns `stopped_by_loop` instead, with the joint whose loop opens and the
fraction reached. When both apply, the motion stops at whichever comes first, and only that
one is reported. The Drive Joints dialog shows the reached value and the reason while it
previews, and the status line repeats it after OK.

The trajectory uses bounded samples of cached tessellated geometry. Spacing accounts for
effective joint travel, angular motion, distance from joint pivots and whole-body widths
along sampled face directions. This handles tilted thin plates while keeping curved bodies
practical to drive. It is not exact continuous collision detection: locally thin walls in
a larger body, unsampled face directions and very small geometric details remain limitations.

When a motion or assembly exceeds the sampling, geometry or collision-work budget, the
command returns an error and restores the document. Use smaller drive increments or a
smaller contact set. Mesh bodies in contact sets are currently rejected for swept driving.
An assembly already overlapping at the start retains the recovery behaviour that allows
it to move out of that invalid placement. Disable contact to move freely.

`motion.export` uses the same checks between its requested poses, including sparse exports,
and leaves the source document unchanged. Contact checks interpolate joint values between
those poses; they do not reconstruct intermediate motion-study keyframes omitted by the
requested sample steps.
