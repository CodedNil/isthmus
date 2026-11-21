# Isthmus

Write GPU graphics in Rust, alongside the application that drives them.

The renderer uses Vulkan. Cantus, Lares, and Paries run on Linux Wayland.

- **Inline shaders.** A shader is one method chain where you draw: say which variables it needs, give it a shape, and paint it. The closures capture ordinary local variables, and the ones you list are sent to the GPU.
- **Composable 2D graphics.** `isthmus_2d` provides shapes, smooth unions, cutouts, outlines and scalable text. Custom distance functions work with the same antialiasing and painting tools.
- **Shared Rust.** Reuse types and helpers across CPU and GPU code, with checked data interfaces.
- **Rendering beyond a window.** Draw to images, combine multiple passes, and share a renderer across surfaces.

## An image with a moving, wavy border

Load an image once, then draw it each frame:

```rust
let pixels = image::open("cover.png")?.into_rgba8();
let image = Image::rgba8([pixels.width(), pixels.height()], pixels.into_raw());

// Inside the drawing function:
let image: &Image = &image;
let center: Vec2 = frame.screen_size * 0.5;
let radius: f32 = 80.0;

frame
    .shader("wavy cover")
    .with((image, center, radius))
    .shape(|frame| {
        let bounds = Rect::from_center_size(center, Vec2::splat((radius + 6.0) * 2.0));
        Shape::from_fn(bounds, move |point| {
            let p = point - center;
            let angle = if p.length_squared() > 1.0 { p.y.atan2(p.x) } else { 0.0 };
            let wave = (angle * 8.0 - frame.time * 2.0).sin() * 6.0;
            p.length() - radius - wave
        })
        .outlined(3.0)
    })
    .paint(|_, fragment| {
        let uv = (fragment.pixel - center) / (radius * 2.0) + 0.5;
        fragment.paint(image.sample(uv), vec4(0.9, 0.7, 1.0, 1.0))
    });
```

The image fills an animated circle, with an antialiased lavender outline. Each stage says where it runs:

- `frame.shader("name")` starts a draw. The name keys the shader and labels it on the GPU and in logs. `.blend(..)` and `.format(..)` are ordinary methods on it.
- `.with((..))` lists the local variables the closures use, which are sent to the GPU. The build checks that the list matches the closures' captures exactly, and reads each variable's type from its annotation, so the GPU knows it too. The CPU can still use them normally.
- `.shape(..)` is the geometry. The CPU evaluates it to place the draw, and the GPU evaluates it to cover each pixel.
- `.paint(..)` is the colour. It runs on the GPU, once per covered pixel.

## Text uses the same drawing tools

Install fonts once with `install_fonts`. A `Line` is shaped text, cached and independent of any renderer, and `at` places it. Send the result like any other value:

```rust
let line = Line::new("Hello, GPU", 28.0, 700.0);
let title: Text = line.at(Vec2::ZERO).centered(frame.screen_size * 0.5);

frame.shader("title").with(title).shape(|_| title.outlined(2.0)).paint(|frame, fragment| {
    let t = (fragment.pixel.x * 0.02 - frame.time).sin() * 0.5 + 0.5;
    let fill = vec3(0.2, 0.7, 1.0).lerp(vec3(1.0, 0.4, 0.7), t);
    fragment.paint(fill.extend(1.0), vec4(0.05, 0.05, 0.1, 1.0))
});
```

The glyphs are written to the frame when the text is sent, once however many draws use the same line.

## Principles

- Make the simplest correct program the easiest program to write.
- Make invalid CPU/GPU interfaces impossible to build, not errors discovered while rendering.
- Use immediate-mode drawing: describe each frame, while Isthmus derives layouts, bindings and GPU work from intent.
- Keep related CPU and GPU behaviour together as ordinary, readable, rustfmt-friendly Rust.
- Remain platform and window-system agnostic.
- Provide escape hatches without making unusual cases define the common API.
- Prefer one composable concept over several overlapping conveniences.
- Add abstraction only when it removes more complexity from users than it introduces into Isthmus.
- Treat runtime cost, memory cost, compile time and diagnostics as parts of ergonomics, not afterthoughts.
