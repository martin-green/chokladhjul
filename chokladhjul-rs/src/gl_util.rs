//! Thin helpers on top of glow: shader programs, textures and a streaming quad buffer.

use std::cell::RefCell;
use std::collections::HashMap;

use glow::HasContext;

pub struct Program {
    pub id: glow::Program,
    locations: RefCell<HashMap<&'static str, Option<glow::UniformLocation>>>,
}

impl Program {
    pub fn new(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, src);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(format!("shader compile error: {}", gl.get_shader_info_log(shader)));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.bind_attrib_location(program, 1, "a_uv");
            gl.bind_attrib_location(program, 2, "a_color");
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!("program link error: {}", gl.get_program_info_log(program)));
            }
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }
            Ok(Program { id: program, locations: RefCell::new(HashMap::new()) })
        }
    }

    fn loc(&self, gl: &glow::Context, name: &'static str) -> Option<glow::UniformLocation> {
        *self.locations
            .borrow_mut()
            .entry(name)
            .or_insert_with(|| unsafe { gl.get_uniform_location(self.id, name) })
    }

    pub fn bind(&self, gl: &glow::Context) {
        unsafe { gl.use_program(Some(self.id)) }
    }

    pub fn set_f32(&self, gl: &glow::Context, name: &'static str, v: f32) {
        unsafe { gl.uniform_1_f32(self.loc(gl, name).as_ref(), v) }
    }

    pub fn set_i32(&self, gl: &glow::Context, name: &'static str, v: i32) {
        unsafe { gl.uniform_1_i32(self.loc(gl, name).as_ref(), v) }
    }

    pub fn set_vec2(&self, gl: &glow::Context, name: &'static str, v: [f32; 2]) {
        unsafe { gl.uniform_2_f32(self.loc(gl, name).as_ref(), v[0], v[1]) }
    }

    pub fn set_vec3(&self, gl: &glow::Context, name: &'static str, v: [f32; 3]) {
        unsafe { gl.uniform_3_f32(self.loc(gl, name).as_ref(), v[0], v[1], v[2]) }
    }

    pub fn set_vec4(&self, gl: &glow::Context, name: &'static str, v: [f32; 4]) {
        unsafe { gl.uniform_4_f32(self.loc(gl, name).as_ref(), v[0], v[1], v[2], v[3]) }
    }
}

#[derive(Clone, Copy)]
pub struct Texture {
    pub id: glow::Texture,
    pub width: u32,
    pub height: u32,
}

impl Texture {
    /// Uploads premultiplied RGBA8 pixels.
    pub fn rgba(gl: &glow::Context, width: u32, height: u32, pixels: &[u8], mipmaps: bool) -> Result<Self, String> {
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        Self::upload(gl, width, height, pixels, glow::RGBA8, glow::RGBA, mipmaps)
    }

    /// Single channel texture (used for the text distance field).
    pub fn r8(gl: &glow::Context, width: u32, height: u32, pixels: &[u8]) -> Result<Self, String> {
        assert_eq!(pixels.len(), (width * height) as usize);
        Self::upload(gl, width, height, pixels, glow::R8, glow::RED, false)
    }

    fn upload(
        gl: &glow::Context,
        width: u32,
        height: u32,
        pixels: &[u8],
        internal: u32,
        format: u32,
        mipmaps: bool,
    ) -> Result<Self, String> {
        unsafe {
            let id = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(id));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal as i32,
                width as i32,
                height as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(pixels)),
            );
            let min = if mipmaps { glow::LINEAR_MIPMAP_LINEAR } else { glow::LINEAR };
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            if mipmaps {
                gl.generate_mipmap(glow::TEXTURE_2D);
            }
            Ok(Texture { id, width, height })
        }
    }

    pub fn bind(&self, gl: &glow::Context, unit: u32) {
        unsafe {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.id));
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

pub type Color = [f32; 4];

/// Collects quads on the CPU and streams them to the GPU when flushed. The
/// caller binds the program, uniforms and textures before flushing.
pub struct QuadBatch {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    capacity_bytes: usize,
    verts: Vec<Vertex>,
}

impl QuadBatch {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = std::mem::size_of::<Vertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, stride, 16);
            gl.bind_vertex_array(None);
            Ok(QuadBatch { vao, vbo, capacity_bytes: 0, verts: Vec::with_capacity(6 * 1024) })
        }
    }

    /// Corners in order top-left, top-right, bottom-right, bottom-left.
    pub fn quad(&mut self, pos: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: [Color; 4]) {
        for i in [0, 1, 2, 0, 2, 3] {
            self.verts.push(Vertex { pos: pos[i], uv: uv[i], color: color[i] });
        }
    }

    /// Axis-aligned or rotated sprite. `pivot` is in 0..1 sprite space and
    /// lands on `at`; rotation is clockwise in radians.
    #[allow(clippy::too_many_arguments)]
    pub fn sprite(&mut self, at: [f32; 2], size: [f32; 2], pivot: [f32; 2], rotation: f32, uv: [f32; 4], color: Color) {
        let (s, c) = rotation.sin_cos();
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let pos = corners.map(|k| {
            let x = (k[0] - pivot[0]) * size[0];
            let y = (k[1] - pivot[1]) * size[1];
            [at[0] + x * c - y * s, at[1] + x * s + y * c]
        });
        let uvs = [[uv[0], uv[1]], [uv[2], uv[1]], [uv[2], uv[3]], [uv[0], uv[3]]];
        self.quad(pos, uvs, [color; 4]);
    }

    /// Centered, unrotated sprite using the full texture.
    pub fn centered(&mut self, at: [f32; 2], size: [f32; 2], color: Color) {
        self.sprite(at, size, [0.5, 0.5], 0.0, [0.0, 0.0, 1.0, 1.0], color);
    }

    pub fn flush(&mut self, gl: &glow::Context) {
        if self.verts.is_empty() {
            return;
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(
                self.verts.as_ptr() as *const u8,
                self.verts.len() * std::mem::size_of::<Vertex>(),
            )
        };
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            if bytes.len() > self.capacity_bytes {
                self.capacity_bytes = bytes.len().next_power_of_two();
                gl.buffer_data_size(glow::ARRAY_BUFFER, self.capacity_bytes as i32, glow::STREAM_DRAW);
            } else {
                // Orphan the old storage so the driver does not stall on the previous draw.
                gl.buffer_data_size(glow::ARRAY_BUFFER, self.capacity_bytes as i32, glow::STREAM_DRAW);
            }
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
            gl.draw_arrays(glow::TRIANGLES, 0, self.verts.len() as i32);
        }
        self.verts.clear();
    }
}

/// A vertex array without attributes, for full screen passes that build
/// their vertices from gl_VertexID.
pub struct EmptyVao(pub glow::VertexArray);

impl EmptyVao {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe { Ok(EmptyVao(gl.create_vertex_array()?)) }
    }

    pub fn draw_fullscreen(&self, gl: &glow::Context) {
        unsafe {
            gl.bind_vertex_array(Some(self.0));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }
}
