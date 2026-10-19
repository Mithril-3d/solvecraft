//! A sketch line can be `infinite` (#1359): construction geometry through its two points, drawn
//! across the whole view, a boundary for trim and extend along its whole line. Saved only when
//! set: older designs need no transformation, and a line without it is the line it always was;
//! 0.13.x (which reads up to 24) asks for an update instead of drawing such a line between its
//! two points.
