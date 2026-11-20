//! GLSL 3.30 shaders. Everything works in a 1920x1080 virtual space that
//! `u_view` maps to the window; all colors are premultiplied alpha.

pub const QUAD_VS: &str = r#"#version 330 core
in vec2 a_pos;
in vec2 a_uv;
in vec4 a_color;
uniform vec4 u_view;
out vec2 v_uv;
out vec4 v_color;
void main() {
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = vec4(a_pos.x * u_view.x + u_view.z, a_pos.y * u_view.y + u_view.w, 0.0, 1.0);
}
"#;

pub const SPRITE_FS: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;
out vec4 frag;
uniform sampler2D u_tex;
void main() {
    frag = texture(u_tex, v_uv) * v_color;
}
"#;

/// Rotating disc/arrow with angular motion blur, winner spotlight and gloss.
pub const WHEEL_FS: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;
out vec4 frag;
uniform sampler2D u_tex;
uniform float u_angle;      // clockwise rotation, radians
uniform float u_blur;       // rotation covered by the virtual shutter, radians
uniform float u_radius_px;  // on-screen radius in pixels
uniform float u_base_lod;   // mip level matching the on-screen size
uniform float u_lod_bias;
uniform float u_sectors;
uniform float u_win;        // winning sector, < 0 for none
uniform float u_highlight;  // 0..1 spotlight amount
uniform float u_hub;        // radius fraction that is never dimmed
uniform float u_time;
uniform float u_gloss;
uniform float u_shadow;     // 1 = soft black silhouette

const float PI = 3.14159265;

// Screen space -> texture space (rotate by -a).
vec2 unrotate(vec2 p, float a) {
    float s = sin(a);
    float c = cos(a);
    return vec2(p.x * c + p.y * s, -p.x * s + p.y * c);
}

float noise(vec2 p) {
    return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715))));
}

void main() {
    vec2 p = v_uv - 0.5;
    float r = length(p) * 2.0;
    if (r > 1.0) discard;

    float arc = r * u_radius_px * abs(u_blur);
    float taps = clamp(ceil(arc / 3.5), 1.0, 24.0);
    float lod = u_base_lod + u_lod_bias + clamp(log2(max(arc / taps, 1.0)) - 0.5, 0.0, 2.5);

    vec4 acc;
    if (taps < 1.5) {
        acc = textureLod(u_tex, unrotate(p, u_angle) + 0.5, lod);
    } else {
        // Rotate incrementally from tap to tap instead of calling sin/cos per tap.
        acc = vec4(0.0);
        float step = u_blur / taps;
        vec2 q = unrotate(p, u_angle - step * noise(gl_FragCoord.xy));
        float cs = cos(step);
        float sn = sin(step);
        int n = int(taps);
        for (int i = 0; i < n; i++) {
            acc += textureLod(u_tex, q + 0.5, lod);
            q = vec2(q.x * cs - q.y * sn, q.x * sn + q.y * cs);
        }
        acc /= taps;
    }

    if (u_win >= 0.0 && u_highlight > 0.001) {
        vec2 q = unrotate(p, u_angle);
        float a = atan(q.x, -q.y);
        float w = 2.0 * PI / u_sectors;
        float d = abs(mod(a - u_win * w + PI, 2.0 * PI) - PI);
        float half_w = 0.5 * w;
        float arc_px = max(r * u_radius_px, 1.0);
        float inside = 1.0 - smoothstep(half_w - 1.2 / arc_px, half_w + 1.2 / arc_px, d);
        float zone = smoothstep(u_hub, u_hub + 0.02, r);
        acc.rgb *= 1.0 - (1.0 - inside) * zone * 0.6 * u_highlight;
        float pulse = 0.5 + 0.5 * sin(u_time * 5.0);
        acc.rgb += inside * zone * u_highlight * (0.08 + 0.12 * pulse) * acc.a;
        float edge_px = (half_w - d) * arc_px;
        float border = exp(-edge_px * edge_px / 10.0) * inside * zone;
        acc.rgb += vec3(1.0, 0.86, 0.35) * border * u_highlight * (0.6 + 0.4 * pulse) * acc.a;
    }

    if (u_gloss > 0.0) {
        float g = 1.0 - smoothstep(0.0, 0.6, length((p - vec2(-0.17, -0.21)) * vec2(1.0, 1.4)));
        acc.rgb += vec3(g * g * 0.16 * u_gloss) * acc.a;
        acc.rgb *= 1.0 - 0.16 * smoothstep(0.72, 1.0, r) * u_gloss;
    }

    if (u_shadow > 0.5) {
        acc = vec4(0.0, 0.0, 0.0, acc.a);
    }
    frag = acc * v_color;
}
"#;

/// Signed distance field text with outline and soft shadow support.
pub const TEXT_FS: &str = r#"#version 330 core
in vec2 v_uv;
in vec4 v_color;
out vec4 frag;
uniform sampler2D u_tex;
uniform float u_px_per_unit;   // screen pixels per unit of stored distance
uniform float u_outline;       // outline width in pixels
uniform vec4 u_outline_color;
uniform float u_soft;          // edge softness in pixels (0 = crisp)
void main() {
    float d = (texture(u_tex, v_uv).r - 0.5) * u_px_per_unit;
    float aa = max(u_soft, 0.7);
    float outer = smoothstep(-aa, aa, d + u_outline);
    float inner = smoothstep(-aa, aa, d - 0.25);
    frag = mix(u_outline_color, v_color, inner) * outer;
}
"#;

pub const FULLSCREEN_VS: &str = r#"#version 330 core
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

pub const BACKGROUND_FS: &str = r#"#version 330 core
out vec4 frag;
uniform vec2 u_res;            // window size in pixels
uniform vec3 u_virtual;        // scale, offset x, offset y (virtual -> window pixels)
uniform float u_time;
uniform vec2 u_ray_center;
uniform float u_ray_strength;
uniform vec2 u_burst_center;
uniform float u_burst;
uniform vec3 u_burst_color;

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

void main() {
    vec2 px = vec2(gl_FragCoord.x, u_res.y - gl_FragCoord.y);
    vec2 v = (px - u_virtual.yz) / u_virtual.x;

    // Same gradient as the original web version.
    vec3 col = mix(vec3(0.067, 0.333, 0.467), vec3(0.0, 0.467, 0.267), clamp(px.y / u_res.y, 0.0, 1.0));

    vec2 d = v - u_ray_center;
    float r = length(d);
    float a = atan(d.y, d.x);
    float rays = smoothstep(0.35, 1.0, 0.5 + 0.5 * sin(a * 16.0 + u_time * 0.12));
    col += vec3(0.85, 1.0, 0.9) * rays * smoothstep(1500.0, 420.0, r) * u_ray_strength;
    col += vec3(0.12, 0.2, 0.16) * exp(-r * r / (650.0 * 650.0));

    if (u_burst > 0.001) {
        vec2 bd = v - u_burst_center;
        float br = length(bd);
        float ba = atan(bd.y, bd.x);
        float b = pow(0.5 + 0.5 * sin(ba * 12.0 - u_time * 0.7), 3.0);
        col += u_burst_color * b * smoothstep(1000.0, 260.0, br) * u_burst * 0.45;
        col += u_burst_color * exp(-br * br / (380.0 * 380.0)) * u_burst * 0.3;
    }

    vec2 uv = px / u_res - 0.5;
    col *= mix(0.62, 1.0, smoothstep(0.95, 0.25, length(uv * vec2(1.0, 1.25))));
    col += (hash(px + fract(u_time) * 17.0) - 0.5) / 255.0;
    frag = vec4(col, 1.0);
}
"#;
