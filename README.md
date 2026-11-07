# Isthmus

Write GPU graphics in Rust, alongside the application that drives them.

- **Inline shaders.** `shader!` pairs a primitive with the code that colours it. Capture typed values, images and slices directly from your drawing code.
- **Composable 2D graphics.** `isthmus_2d` provides shapes, smooth unions, cutouts, outlines and scalable text. Custom distance functions work with the same antialiasing and painting tools.
- **Shared Rust.** Reuse types and helpers across CPU and GPU code, with checked data interfaces.
- **Rendering beyond a window.** Draw to images, combine multiple passes, and share a renderer across surfaces on native platforms and the web.

## An image with a moving, wavy border

Load an image once, then draw it each frame:

```rust
let pixels = image::open("cover.png")?.into_rgba8();
let image = Image::rgba8([pixels.width(), pixels.height()], pixels.into_raw());

// Inside the drawing function:
let image = &image;
let center = frame.screen_size * 0.5;
let radius = 80.0;
shader!(
    frame,
    |frame, center: Vec2, radius: f32, image: &Image| {
        let bounds = Rect::from_center_size(center, Vec2::splat((radius + 6.0) * 2.0));
        Shape::from_fn(bounds, move |point| {
            let p = point - center;
            let angle = if p.length_squared() > 1.0 { p.y.atan2(p.x) } else { 0.0 };
            let wave = (angle * 8.0 - frame.time * 2.0).sin() * 6.0;
            p.length() - radius - wave
        }).outlined(3.0)
    },
    |_, fragment| {
        let uv = (fragment.pixel - center) / (radius * 2.0) + 0.5;
        fragment.paint(image.sample(uv), vec4(0.9, 0.7, 1.0, 1.0))
    },
);
```

The image fills an animated circle, with an antialiased lavender outline. The first closure defines the shape; the second paints it.

## Text uses the same drawing tools

With a `TextCache` as the program's resources:

```rust
let title = frame.resources
    .line("Hello, GPU", 28.0, 700.0)
    .centered(frame.screen_size * 0.5)
    .outlined(2.0);
shader!(
    frame,
    |_, title: Text| title,
    |frame, fragment| {
        let t = (fragment.pixel.x * 0.02 - frame.time).sin() * 0.5 + 0.5;
        let fill = vec3(0.2, 0.7, 1.0).lerp(vec3(1.0, 0.4, 0.7), t);
        fragment.paint(fill.extend(1.0), vec4(0.05, 0.05, 0.1, 1.0))
    },
);
```

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
