//! A motion link can carry a `coupling`: a gear pair (`ra`, `rb`, `internal`), a rack and pinion
//! (`radius`) or a screw (`pitch`), each size a number in mm, an expression or a picked round
//! face, and `reverse`. The link's `ratio` is worked out from it whenever the design changes and
//! saved as well, so a link reads the same without it. Version 22 designs need no
//! transformation: a link without `coupling` is a plain motion link, as it always was; 0.13.0
//! (which reads up to 22) asks for an update instead of keeping a gear's ratio when its radius
//! changes.
