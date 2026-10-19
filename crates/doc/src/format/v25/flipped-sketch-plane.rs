//! A sketch can sit on a plane seen from its other side: `{"type": "flipped", "base": <plane>}`
//! (with `"about_y": true` when it turns about the plane's y axis rather than its x axis). New
//! sketches started from behind an origin or construction plane save this, so they face the
//! viewer. Older designs have no flipped planes and open unchanged; the last release refuses a
//! design that has one (it asks for an update) instead of putting the sketch on the wrong side.
