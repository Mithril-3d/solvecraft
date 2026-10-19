//! Sketch linear patterns save two independent live direction-line references and each
//! direction's alignment and reversal. Even centered totals keep the original fixed and use
//! reversal to choose which side receives the extra copy. Older patterns retain their fixed
//! vectors and positive-direction placement through defaulted fields, so no rewrite is needed.
//! The last release refuses this pending format instead of silently losing the references.
