//! A Split Body feature can take a straight sketch line as its tool (`sketch`, the sketch's
//! feature id, and `line`, the line's id), splitting by the plane through the line along the
//! sketch's normal (#1552). Older designs need no transformation: a split without them uses its
//! plane or face as before; 0.13.x asks for an update instead of splitting by the stored plane.
