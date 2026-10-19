//! An explicit plane (a mirror's, or any plane reference) can be `placed`: an `origin` of three
//! length expressions with `x_dir` and `y_dir`, so it follows the design's parameters (#1492). A
//! plane whose origin is plain numbers is still saved as `custom`. Older designs need no
//! transformation; 0.13.x asks for an update instead of failing to read the plane.
