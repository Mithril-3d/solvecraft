//! A browser folder or group can be hidden on its own (`visibility.hidden_folders`, tree keys
//! such as `c0/bodies`, `o:4/sketches` or `c0/g7`): its items keep their own hidden state and
//! are drawn again as they were once it is shown. Older designs need no transformation: they
//! hide no folder, and a folder's eye then follows its own state. The last release asks for an
//! update instead of opening a design with this format number.
