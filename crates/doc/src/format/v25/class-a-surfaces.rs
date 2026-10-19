//! The Class-A surface pass adds `SurfaceBoundary` (Boundary Surface: `sketch`, `curves`,
//! optional `orders`, `crown`, `samples`), `SurfaceMatch` (Match Surface: `body`, optional
//! `faces`, `target`, `edges`, optional `order`) and `SurfaceFair` (Fair Surface: `body`, optional
//! `faces`, `strength`, `iterations`, `keep_edges`, `tolerance`). Older designs need no
//! transformation; 0.13.x asks for an update instead of meeting a feature it can't read.
