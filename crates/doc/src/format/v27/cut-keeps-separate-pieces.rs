//! A cut keeps a body that is several separate pieces (posts joined while apart) as one body; it
//! splits off only the pieces the cut itself made (#2247). Older designs need no transformation:
//! where they named a piece the cut used to split off next to its body in a combine (tools
//! `Posts`, `Posts (1)`), the combine takes `Posts` and warns that the piece is part of it. 0.15.0
//! asks for an update instead of splitting the pieces apart again.
