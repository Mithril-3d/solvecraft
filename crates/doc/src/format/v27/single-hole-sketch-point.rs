//! A Single hole picked at a sketch point keeps that point (`points` with `single: true`) and is
//! drilled where the point is after the sketch changes, along its own direction into the face
//! (#2110). Older designs need no transformation: a Single hole without it stays at the position
//! it was placed at, as before; 0.15.x asks for an update instead of opening one that has it.
