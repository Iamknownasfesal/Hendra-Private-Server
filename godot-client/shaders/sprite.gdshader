shader_type spatial;
render_mode unshaded, cull_disabled, depth_draw_always, shadows_disabled;

// Sprite rendering: outline, dye recolouring and condition tints, all done on the GPU.
//
// The AS3 client produced these effects by baking new bitmaps. Every sprite was upscaled five
// times, run through a Pixel Bender shader to swap in dye textures, given a black outline with a
// glow filter, and then stored in a dictionary keyed by the source bitmap. There were eight such
// caches, most of them never cleared for the lifetime of the process, and a texture had to be
// rebuilt whenever a character changed size, dye, or status effect.
//
// All of it collapses into this. Nothing is allocated at runtime and nothing is cached.
//
// Every sprite on a sheet is one instance of the same unit quad. What differs between them arrives
// per instance:
//   INSTANCE_CUSTOM - the sprite's rectangle within the sheet: origin in xy, size in zw
//   COLOR           - tint and alpha
//   the transform   - where the quad lands, how big, which way up, and mirrored or not
//
// Sending four corners each instead meant roughly sixty floats a sprite crossing to the GPU every
// frame, freshly allocated, where an instance is twenty and the buffer is reused. Knowing the
// rectangle is also what makes the outline safe: every sprite shares a sheet with hundreds of
// others, so sampling a neighbouring texel without clamping would pull in whatever sits next door
// on the atlas.

uniform sampler2D sheet : source_color, filter_nearest;

// Recolour sources. The mask's red channel selects the first, green the second, matching how the
// original's dye masks were authored.
uniform sampler2D dye_mask : source_color, filter_nearest, hint_default_transparent;
uniform bool use_dye_mask = false;
uniform vec4 dye_one : source_color = vec4(0.0);
uniform vec4 dye_two : source_color = vec4(0.0);

uniform vec4 outline_colour : source_color = vec4(0.0, 0.0, 0.0, 1.0);

// Outline thickness in screen pixels.
//
// Screen space rather than texture space, because the original's outline is roughly a quarter of a
// source pixel -- it applied a fixed 1.4-pixel blur to a five-times-upscaled bitmap. A dilate
// measured in texels cannot express that: at nearest-neighbour sampling anything under one texel
// lands back on the texel it started from and produces no outline at all, while one full texel is
// six screen pixels of black. Deriving the offset from the UV gradient gives a constant thickness
// on screen no matter how large the sprite is drawn or how far the camera is zoomed.
uniform float outline_pixels = 2.0;

// 0 none, 1 greyscale (paused, stasis, petrified), 2 red (cursed).
uniform int tint_mode = 0;

// The original darkened every sprite slightly from top to bottom before outlining it. Subtle, but
// its absence is noticeable across a whole screen.
uniform float vertical_shade = 0.16;

varying vec2 region_min;
varying vec2 region_size;
varying vec4 tint;

void vertex() {
    region_min = INSTANCE_CUSTOM.xy;
    region_size = INSTANCE_CUSTOM.zw;
    tint = COLOR;

    // The quad carries its own corner as a fraction of the sprite -- nought to one, or a little
    // outside it where there is an outline to draw into -- and the rectangle turns that into a
    // place on the sheet.
    UV = region_min + UV * region_size;
}

// Samples the sheet, treating anything outside this sprite's own rectangle as empty.
vec4 sample_sprite(vec2 uv) {
    vec2 local = (uv - region_min) / region_size;
    if (local.x < 0.0 || local.x > 1.0 || local.y < 0.0 || local.y > 1.0) {
        return vec4(0.0);
    }
    return texture(sheet, uv);
}

void fragment() {
    // Godot does not allow an early return from the fragment processor, so both the sprite and the
    // outline case are computed into these and chosen between at the end.
    vec3 out_albedo;
    float out_alpha;

    vec4 colour = sample_sprite(UV);

    // The half-alpha test decides where the outline goes, so it only applies when there is one.
    // Without this second clause anything soft-edged -- a shadow, a particle's glow -- would be cut
    // off hard at the radius where its alpha crosses a half, since every texel below that would
    // fall into the outline branch and be discarded for having no solid neighbour.
    if (colour.a > 0.5 || outline_pixels <= 0.0) {
        if (colour.a <= 0.0) {
            discard;
        }

        if (use_dye_mask) {
            vec4 mask = texture(dye_mask, UV);
            colour.rgb = mix(colour.rgb, dye_one.rgb, mask.r * dye_one.a);
            colour.rgb = mix(colour.rgb, dye_two.rgb, mask.g * dye_two.a);
        }

        // Local vertical position within the sprite, so the gradient does not shift as the sprite
        // moves across the sheet.
        float down = clamp((UV.y - region_min.y) / region_size.y, 0.0, 1.0);
        colour.rgb *= 1.0 - vertical_shade * down;

        if (tint_mode == 1) {
            float grey = dot(colour.rgb, vec3(0.3, 0.59, 0.11));
            colour.rgb = vec3(grey);
        } else if (tint_mode == 2) {
            float grey = dot(colour.rgb, vec3(0.3, 0.59, 0.11));
            colour.rgb = vec3(grey, 0.0, 0.0);
        }

        out_albedo = colour.rgb * tint.rgb;
        out_alpha = colour.a * tint.a;
    } else {
        // Transparent here, so this texel becomes outline if any neighbour is solid. Eight samples
        // rather than four, otherwise diagonal edges come out ragged.
        vec2 offset = fwidth(UV) * outline_pixels;
        float neighbour = 0.0;
        neighbour = max(neighbour, sample_sprite(UV + vec2(offset.x, 0.0)).a);
        neighbour = max(neighbour, sample_sprite(UV - vec2(offset.x, 0.0)).a);
        neighbour = max(neighbour, sample_sprite(UV + vec2(0.0, offset.y)).a);
        neighbour = max(neighbour, sample_sprite(UV - vec2(0.0, offset.y)).a);
        neighbour = max(neighbour, sample_sprite(UV + offset).a);
        neighbour = max(neighbour, sample_sprite(UV - offset).a);
        neighbour = max(neighbour, sample_sprite(UV + vec2(offset.x, -offset.y)).a);
        neighbour = max(neighbour, sample_sprite(UV + vec2(-offset.x, offset.y)).a);

        if (neighbour < 0.5) {
            discard;
        }

        out_albedo = outline_colour.rgb;
        out_alpha = outline_colour.a * tint.a;
    }

    ALBEDO = out_albedo;
    ALPHA = out_alpha;
}
