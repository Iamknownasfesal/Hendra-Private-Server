shader_type spatial;
render_mode unshaded, cull_disabled, shadows_disabled;

// Terrain rendering. The only thing here that is not trivial is the wrapping, and it is the whole
// reason this shader exists rather than a StandardMaterial3D.
//
// Animated terrain -- flowing water, lava, conveyor belts -- scrolls by sliding its texture
// coordinates. The original could do that with a plain offset because every tile was its own small
// bitmap with repeat switched on, so the slide wrapped around inside it. Here every tile is one
// rectangle of a shared sheet, and sliding past the edge of that rectangle samples whatever sprite
// happens to sit next door on the atlas: a river with somebody's dirt path scrolling through it.
//
// So the coordinate is wrapped explicitly, within the tile's own rectangle. UV arrives already
// offset; the fractional part of its position inside the rectangle is what gets sampled.
//
// Per-vertex inputs beyond the usual:
//   COLOR - tint
//   UV2   - the origin of this tile's rectangle within the sheet
//
// The rectangle's size is a uniform, because surfaces are grouped by it anyway -- the same
// arrangement the sprite shader uses.

uniform sampler2D sheet : source_color, filter_nearest;

// Size of one tile's rectangle, in texture coordinates.
uniform vec2 region_size = vec2(1.0);

varying vec2 region_min;
varying vec4 tint;

void vertex() {
    region_min = UV2;
    tint = COLOR;
}

void fragment() {
    vec2 local = fract((UV - region_min) / region_size);
    vec4 colour = texture(sheet, region_min + local * region_size);

    // ALPHA is deliberately not written. Terrain is opaque, and writing it would move the whole
    // ground into the transparent pass, behind the depth-sorted objects standing on it.
    ALBEDO = colour.rgb * tint.rgb;
}
