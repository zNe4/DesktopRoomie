struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) phase: u32,
}

@vertex
fn vertex_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) phase: u32) -> VertexOutput {
    let triangle = array(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return VertexOutput(vec4(triangle[vertex], 0.0, 1.0), phase);
}

fn inside(p: vec2<u32>, start: vec2<u32>, end: vec2<u32>) -> bool {
    return all(p >= start) && all(p < end);
}

fn from_bgra(bgra: vec4<u32>) -> vec4<f32> {
    // Shader output is semantic RGBA. The Bgra8Unorm attachment stores BGRA.
    return vec4<f32>(bgra.zyxw) / 255.0;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let p = vec2<u32>(input.position.xy);
    switch input.phase {
        case 0u, 1u: {
            if inside(p, vec2(17u, 40u), vec2(73u, 96u)) {
                return from_bgra(select(vec4(230u, 115u, 60u, 255u), vec4(70u, 211u, 150u, 255u), input.phase == 1u));
            }
        }
        case 2u: {
            if inside(p, vec2(51u, 53u), vec2(105u, 110u)) && !inside(p, vec2(61u, 65u), vec2(86u, 96u)) {
                return from_bgra(vec4(112u, 45u, 96u, 128u));
            }
        }
        case 3u: {
            if inside(p, vec2(113u, 24u), vec2(137u, 58u)) {
                return from_bgra(vec4(90u, 160u, 240u, 255u));
            }
            if inside(p, vec2(132u, 101u), vec2(153u, 131u)) {
                return from_bgra(vec4(35u, 95u, 120u, 128u));
            }
        }
        default: {}
    }
    discard;
}
