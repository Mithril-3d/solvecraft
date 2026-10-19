//! An Edge Flange feature can choose its own corner relief (`corner_relief_shape`: notch, round,
//! rectangular or tear, and `corner_relief`, the size across), for the corners its flanges make.
//! Version 23 designs need no transformation: a feature without them follows the sheet metal
//! rule, as it always did, and saves as before; 0.13.0 (which reads up to 22) asks for an update
//! instead of silently building the rule's relief where a design asks for another.
