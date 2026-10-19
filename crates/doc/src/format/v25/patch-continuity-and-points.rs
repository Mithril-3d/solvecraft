//! A `Patch` on edges can carry `continuity` (`all`: `g0`, `g1` or `g2`, and `edges`:
//! `[{edge: [x, y, z], g}]` for edges met otherwise; G0 everywhere when absent, and then not
//! saved) and `points` it passes through (none when absent). Older designs need no
//! transformation: a patch without them fills as it always did; a loop that is not a height field
//! over its plane, which 0.13.0 refused, now fills with one face. 0.13.x asks for an update
//! instead of dropping them.
