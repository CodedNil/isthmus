//! Fit no-roll capture cameras to measured architectural image points/lines.
//! `cargo run -p lares --example align -- [view|all] [output-directory] [--write]`
use image::{Rgb, RgbImage};
use isthmus::glam::{DVec2, DVec3};
use lares::{
    home,
    render::{WALL_HEIGHT, WALL_THICKNESS},
};
use measurements::{Feature, VisibleWall};
use std::{env, error::Error, fmt::Write as _, fs, path::Path};
#[path = "support/measurements.rs"]
mod measurements;
#[path = "support/views.rs"]
#[allow(dead_code, reason = "Fitting uses an f64 camera but shares the capture view definitions")]
mod views;

#[derive(Clone, Copy)]
struct Camera([f64; 7]);
impl Camera {
    fn from_view(view: views::View) -> Self {
        let direction = (view.focus - view.eye).normalize();
        Self([
            f64::from(view.eye.x),
            f64::from(view.eye.y),
            f64::from(view.eye.z),
            f64::from(direction.y.atan2(direction.x)),
            f64::from(direction.z.asin()),
            f64::from((view.fov * 0.5).tan()).ln(),
            0.0,
        ])
    }

    fn basis(self) -> (DVec3, DVec3, DVec3) {
        let (sy, cy) = self.0[3].sin_cos();
        let (sp, cp) = self.0[4].sin_cos();
        let forward = DVec3::new(cp * cy, cp * sy, sp);
        let right = DVec3::Z.cross(forward).normalize();
        let up = forward.cross(right);
        let (sin, cos) = self.0[6].sin_cos();
        (forward, right * cos + up * sin, up * cos - right * sin)
    }

    const fn eye(self) -> DVec3 {
        DVec3::new(self.0[0], self.0[1], self.0[2])
    }

    fn project(self, point: DVec3, size: DVec2) -> DVec2 {
        let (forward, right, up) = self.basis();
        let offset = point - self.eye();
        let depth = forward.dot(offset).max(0.03);
        let focal = size.y * 0.5 / self.0[5].exp();
        size * 0.5 + DVec2::new(right.dot(offset), -up.dot(offset)) * (focal / depth)
    }

    fn image_direction(self, axis: DVec3, midpoint: DVec2, size: DVec2) -> DVec2 {
        let (forward, right, up) = self.basis();
        let depth = forward.dot(axis);
        let focal = size.y * 0.5 / self.0[5].exp();
        let homogeneous = size * 0.5 * depth + DVec2::new(right.dot(axis), -up.dot(axis)) * focal;
        (homogeneous - midpoint * depth).normalize()
    }

    fn definition(self, name: &str) -> String {
        let eye = self.eye();
        let focus = eye + self.basis().0;
        format!(
            "View::new(\"{name}\", vec3({:.5}, {:.5}, {:.5}), vec3({:.5}, {:.5}, {:.5})).with_fov({:.5}),",
            eye.x,
            eye.y,
            eye.z,
            focus.x,
            focus.y,
            focus.z,
            2.0 * self.0[5].exp().atan()
        )
    }
}
fn residuals(camera: Camera, features: &[Feature], size: DVec2) -> Vec<f64> {
    let mut result = Vec::new();
    for feature in features {
        if feature.axis {
            let delta = feature.photo[1] - feature.photo[0];
            let normal = DVec2::new(-delta.y, delta.x).normalize();
            let midpoint = (feature.photo[0] + feature.photo[1]) * 0.5;
            let error = camera.image_direction(feature.world[0], midpoint, size).dot(normal) * delta.length() * 0.5;
            result.extend([error, -error]);
        } else if feature.point {
            let delta = camera.project(feature.world[0], size) - feature.photo[0];
            result.extend([delta.x, delta.y]);
        } else {
            // Compare observed endpoints to the projected infinite model line.
            // Extending a short observed edge far outside the photo would
            // amplify measurement/roll errors and bias the camera position.
            let (forward, right, up) = camera.basis();
            let focal = size.y * 0.5 / camera.0[5].exp();
            let project = |point: DVec3| {
                let delta = point - camera.eye();
                let depth = forward.dot(delta);
                DVec3::new(
                    size.x * 0.5 * depth + focal * right.dot(delta),
                    size.y * 0.5 * depth - focal * up.dot(delta),
                    depth,
                )
            };
            let line = project(feature.world[0]).cross(project(feature.world[1]));
            let scale = line.truncate().length().max(1e-12);
            for point in feature.photo {
                result.push(line.dot(point.extend(1.0)) / scale);
            }
        }
    }
    result
}
fn loss(residuals: &[f64]) -> f64 {
    residuals
        .iter()
        .map(|value| {
            let distance = value.abs();
            if distance <= 12.0 { distance * distance } else { 24.0 * distance - 144.0 }
        })
        .sum()
}
fn solve(mut matrix: [[f64; 7]; 7], mut rhs: [f64; 7]) -> Option<[f64; 7]> {
    for column in 0..7 {
        let pivot = (column..7).max_by(|&a, &b| matrix[a][column].abs().total_cmp(&matrix[b][column].abs()))?;
        if matrix[pivot][column].abs() < 1e-12 {
            return None;
        }
        matrix.swap(column, pivot);
        rhs.swap(column, pivot);
        let scale = matrix[column][column];
        for entry in &mut matrix[column][column..] {
            *entry /= scale;
        }
        rhs[column] /= scale;
        for row in 0..7 {
            if row == column {
                continue;
            }
            let factor = matrix[row][column];
            let pivot_row = matrix[column];
            for (entry, &pivot_entry) in matrix[row][column..].iter_mut().zip(&pivot_row[column..]) {
                *entry -= factor * pivot_entry;
            }
            rhs[row] -= factor * rhs[column];
        }
    }
    Some(rhs)
}
struct Architecture {
    origin: DVec2,
    outlines: Vec<Vec<DVec2>>,
    walls: Vec<(DVec3, DVec3)>,
    openings: Vec<(DVec3, DVec3)>,
    visible_walls: Vec<VisibleWall>,
}
impl Architecture {
    fn new() -> Self {
        let rooms = home::rooms();
        let mut result = Self {
            origin: DVec2::ZERO,
            outlines: Vec::new(),
            walls: Vec::new(),
            openings: Vec::new(),
            visible_walls: Vec::new(),
        };
        let mut min = DVec2::splat(f64::INFINITY);
        let mut max = DVec2::splat(f64::NEG_INFINITY);
        for room in rooms {
            let outline: Vec<_> = room.outline.iter().map(|point| (*point + room.position).as_dvec2()).collect();
            for &point in &outline {
                min = min.min(point);
                max = max.max(point);
            }
            for (&start, &end) in outline.iter().zip(outline.iter().cycle().skip(1)).take(outline.len()) {
                let side = if (start.x - end.x).abs() < 1e-6 {
                    if end.y > start.y { 2 } else { 0 }
                } else if end.x > start.x {
                    3
                } else {
                    1
                };
                if room.walls.bits() & (1 << side) == 0 {
                    continue;
                }
                let low = (start.min(end) - f64::from(WALL_THICKNESS) * 0.5).extend(0.0);
                let high = (start.max(end) + f64::from(WALL_THICKNESS) * 0.5).extend(f64::from(WALL_HEIGHT));
                result.walls.push((low, high));
            }
            for opening in room.openings {
                let centre = (opening.pos + room.position.extend(0.0)).as_dvec3();
                let half = opening.size.as_dvec3() * 0.5;
                result.openings.push((centre - half, centre + half));
            }
            result.outlines.push(outline);
        }
        result.origin = (min + max) * 0.5;
        result
    }

    fn author(&self, point: DVec3) -> DVec3 {
        DVec3::new(point.x + self.origin.x, self.origin.y - point.y, point.z)
    }

    fn in_opening(&self, point: DVec3) -> bool {
        self.openings.iter().any(|&(min, max)| point.cmpge(min - 0.001).all() && point.cmple(max + 0.001).all())
    }

    fn valid_eye(&self, eye: DVec3) -> bool {
        let point = self.author(eye);
        if self.walls.iter().any(|&(min, max)| point.cmpgt(min).all() && point.cmplt(max).all())
            && !self.in_opening(point)
        {
            return false;
        }
        self.outlines.iter().any(|outline| {
            let mut inside = false;
            for (&a, &b) in outline.iter().zip(outline.iter().cycle().skip(1)).take(outline.len()) {
                if (a.y > point.y) != (b.y > point.y) && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x {
                    inside = !inside;
                }
            }
            inside
        })
    }

    fn visible(&self, eye: DVec3, target: DVec3) -> bool {
        let origin = self.author(eye);
        let delta = self.author(target) - origin;
        let length = delta.length();
        for &(min, max) in &self.walls {
            let mut enter = 0.0f64;
            let mut exit = 1.0f64;
            for axis in 0..3 {
                if delta[axis].abs() < 1e-10 {
                    if origin[axis] < min[axis] || origin[axis] > max[axis] {
                        enter = 2.0;
                    }
                } else {
                    let a = (min[axis] - origin[axis]) / delta[axis];
                    let b = (max[axis] - origin[axis]) / delta[axis];
                    enter = enter.max(a.min(b));
                    exit = exit.min(a.max(b));
                }
            }
            if enter <= exit && enter * length < length - 0.08 {
                let middle = origin + delta * f64::midpoint(enter, exit);
                if !self.in_opening(middle) {
                    return false;
                }
            }
        }
        true
    }

    fn valid(&self, camera: Camera, features: &[Feature], size: DVec2) -> bool {
        if !self.valid_eye(camera.eye()) {
            return false;
        }
        let (forward, right, up) = camera.basis();
        let focal = size.y * 0.5 / camera.0[5].exp();
        for wall in &self.visible_walls {
            let ray = (forward + right * ((wall.pixel.x - size.x * 0.5) / focal)
                - up * ((wall.pixel.y - size.y * 0.5) / focal))
                .normalize();
            let facing = (camera.eye() - wall.point).dot(wall.normal);
            let denominator = ray.dot(wall.normal);
            if facing <= 0.0 || denominator >= -1e-8 {
                return false;
            }
            let target = camera.eye() - ray * (facing / denominator);
            if !self.visible(camera.eye(), target) {
                return false;
            }
        }
        features.iter().all(|feature| {
            if feature.point {
                return self.visible(camera.eye(), feature.world[0]);
            }
            if feature.axis {
                return true;
            }
            let direction = (feature.world[1] - feature.world[0]).normalize();
            let offset = camera.eye() - feature.world[0];
            // Observed line endpoints must be visible too. Otherwise a pose
            // can align a distant corner perfectly behind a foreground wall.
            feature.photo.iter().all(|pixel| {
                let ray = (forward + right * ((pixel.x - size.x * 0.5) / focal)
                    - up * ((pixel.y - size.y * 0.5) / focal))
                    .normalize();
                let cosine = direction.dot(ray);
                let denominator = 1.0 - cosine * cosine;
                if denominator < 1e-8 {
                    return true;
                }
                let along_ray = (offset.dot(direction) * cosine - offset.dot(ray)) / denominator;
                let along_edge = offset.dot(direction) + along_ray * cosine;
                along_ray > 0.0 && self.visible(camera.eye(), feature.world[0] + direction * along_edge)
            })
        })
    }
}

fn fit(
    initial: Camera,
    features: &[Feature],
    size: DVec2,
    architecture: Option<&Architecture>,
    calibrate_roll: bool,
) -> Camera {
    let orientation_only = features.iter().all(|feature| feature.axis);
    let residuals = |camera: Camera, features: &[Feature], size: DVec2| {
        let mut result = residuals(camera, features, size);
        // Weak priors stabilize frontal/partially occluded views without letting
        // furniture or guessed image points determine an unsupported position.
        for axis in 0..7 {
            result.push((camera.0[axis] - initial.0[axis]) * [20.0, 20.0, 10.0, 1.0, 1.0, 10.0, 1.0][axis]);
        }
        result
    };
    let mut best = initial;
    let mut best_loss = if architecture.is_none_or(|geometry| geometry.valid(initial, features, size)) {
        loss(&residuals(initial, features, size))
    } else {
        f64::INFINITY
    };
    // Several deterministic starts avoid depending on an artist's initial yaw/FOV.
    for start in 0..13 {
        let mut camera = initial;
        if start > 0 {
            let phase = f64::from(start) * 2.399_963;
            if !orientation_only && architecture.is_some_and(|geometry| !geometry.valid(initial, features, size)) {
                let radius = 0.15 + f64::from(start % 3) * 0.10;
                camera.0[0] += phase.cos() * radius;
                camera.0[1] += phase.sin() * radius;
            }
            camera.0[3] += phase.sin() * 0.20;
            camera.0[4] += phase.cos() * 0.15;
            camera.0[5] += (phase * 1.7).sin() * 0.25;
        }
        let mut damping = 0.01;
        for _ in 0..160 {
            let errors = residuals(camera, features, size);
            let mut derivatives = vec![[0.0; 7]; errors.len()];
            for axis in 0..7 {
                let mut perturbed = camera;
                perturbed.0[axis] += 0.00001;
                let shifted = residuals(perturbed, features, size);
                for (index, row) in derivatives.iter_mut().enumerate() {
                    row[axis] = (shifted[index] - errors[index]) / 0.00001;
                }
            }
            let mut matrix = [[0.0; 7]; 7];
            let mut rhs = [0.0; 7];
            for (error, row) in errors.iter().zip(&derivatives) {
                let weight = 12.0 / error.abs().max(12.0);
                for a in 0..7 {
                    rhs[a] -= row[a] * error * weight;
                    for b in 0..7 {
                        matrix[a][b] += row[a] * row[b] * weight;
                    }
                }
            }
            for (axis, row) in matrix.iter_mut().enumerate() {
                row[axis] = row[axis] * (1.0 + damping) + 0.0001;
            }
            let Some(step) = solve(matrix, rhs) else {
                break;
            };
            let mut trial = camera;
            for (value, &delta) in trial.0.iter_mut().zip(&step) {
                *value += delta.clamp(-0.30, 0.30);
            }
            trial.0[0] = trial.0[0].clamp(initial.0[0] - 2.0, initial.0[0] + 2.0);
            trial.0[1] = trial.0[1].clamp(initial.0[1] - 2.0, initial.0[1] + 2.0);
            trial.0[2] = trial.0[2].clamp(0.75, 2.10);
            if orientation_only {
                trial.0[..3].copy_from_slice(&initial.0[..3]);
            }
            if orientation_only || !calibrate_roll {
                trial.0[6] = 0.0;
            }
            trial.0[4] = trial.0[4].clamp(-1.1, 0.4);
            trial.0[5] = trial.0[5].clamp((0.45f64 * 0.5).tan().ln(), (1.8f64 * 0.5).tan().ln());
            trial.0[6] = trial.0[6].clamp(-0.17, 0.17);
            if architecture.is_some_and(|geometry| !geometry.valid(trial, features, size)) {
                damping = (damping * 5.0).min(1e12);
                continue;
            }
            if loss(&residuals(trial, features, size)) < loss(&errors) {
                camera = trial;
                damping = (damping * 0.4).max(1e-8);
                if step.iter().map(|value| value * value).sum::<f64>() < 1e-14 {
                    break;
                }
            } else {
                damping = (damping * 5.0).min(1e12);
            }
        }
        let current_loss = loss(&residuals(camera, features, size));
        if current_loss < best_loss && architecture.is_none_or(|geometry| geometry.valid(camera, features, size)) {
            best_loss = current_loss;
            best = camera;
        }
    }
    best
}
fn draw_line(image: &mut RgbImage, a: DVec2, b: DVec2, color: [u8; 3]) {
    let delta = b - a;
    let steps = delta.abs().max_element().min(10000.0) as usize + 1;
    for step in 0..=steps {
        let pixel = a + delta * (step as f64 / steps as f64);
        let (x, y) = (pixel.x.round() as i32, pixel.y.round() as i32);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if x + dx >= 0 && y + dy >= 0 && x + dx < image.width() as i32 && y + dy < image.height() as i32 {
                    image.put_pixel((x + dx) as u32, (y + dy) as u32, Rgb(color));
                }
            }
        }
    }
}
fn overlay(photo: &RgbImage, camera: Camera, features: &[Feature]) -> RgbImage {
    let mut result = photo.clone();
    let size = DVec2::new(f64::from(photo.width()), f64::from(photo.height()));
    for feature in features {
        if feature.axis {
            let midpoint = (feature.photo[0] + feature.photo[1]) * 0.5;
            let extent = camera.image_direction(feature.world[0], midpoint, size)
                * feature.photo[0].distance(feature.photo[1])
                * 0.5;
            draw_line(&mut result, feature.photo[0], feature.photo[1], [0, 255, 90]);
            draw_line(&mut result, midpoint - extent, midpoint + extent, [255, 50, 220]);
        } else if feature.point {
            for (point, color) in
                [(feature.photo[0], [0, 255, 90]), (camera.project(feature.world[0], size), [255, 50, 220])]
            {
                draw_line(&mut result, point - DVec2::X * 8.0, point + DVec2::X * 8.0, color);
                draw_line(&mut result, point - DVec2::Y * 8.0, point + DVec2::Y * 8.0, color);
            }
        } else {
            draw_line(&mut result, feature.photo[0], feature.photo[1], [0, 255, 90]);
            draw_line(&mut result, camera.project(feature.world[0], size), camera.project(feature.world[1], size), [
                255, 50, 220,
            ]);
        }
    }
    result
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let requested = args.first().map_or("all", String::as_str);
    let output = Path::new(args.get(1).map_or("/tmp/lares-alignment", String::as_str));
    fs::create_dir_all(output)?;
    let references = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/references");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/support/views.rs");
    let mut definitions = fs::read_to_string(&source)?;
    let mut architecture = Architecture::new();
    let mut report = String::from(
        "Architectural camera fitting: green measured, magenta projected. Reference roll is calibrated; written capture cameras have zero roll.\n",
    );
    for view in views::all(0.0) {
        let features = measurements::for_view(view.name);
        if features.is_empty() || (requested != "all" && requested != view.name) {
            continue;
        }
        let photo = image::open(references.join(format!("{}-reference.webp", view.name)))?.to_rgb8();
        let size = DVec2::new(f64::from(photo.width()), f64::from(photo.height()));
        architecture.visible_walls = measurements::visible_walls(view.name);
        let initial = Camera::from_view(view);
        let calibrated = fit(initial, &features, size, Some(&architecture), true);
        let fitted = fit(initial, &features, size, Some(&architecture), false);
        let before = residuals(initial, &features, size);
        let after = residuals(fitted, &features, size);

        let rms = |values: &[f64]| (values.iter().map(|value| value * value).sum::<f64>() / values.len() as f64).sqrt();
        writeln!(report, "\n{}: RMS {:.2} -> {:.2} reference pixels", view.name, rms(&before), rms(&after))?;
        writeln!(
            report,
            "  reference roll {:.2} deg; physical camera {}",
            calibrated.0[6].to_degrees(),
            architecture.valid(fitted, &features, size)
        )?;
        writeln!(report, "  roll-calibrated diagnostic RMS {:.2}px", rms(&residuals(calibrated, &features, size)))?;
        writeln!(report, "{}", fitted.definition(view.name))?;
        for (feature, values) in features.iter().zip(after.as_chunks::<2>().0) {
            writeln!(report, "  {}: {:.1}, {:.1}px", feature.label, values[0], values[1])?;
            if !feature.point {
                let observed = (feature.photo[1] - feature.photo[0]).normalize();
                let projected = if feature.axis {
                    fitted.image_direction(feature.world[0], (feature.photo[0] + feature.photo[1]) * 0.5, size)
                } else {
                    (fitted.project(feature.world[1], size) - fitted.project(feature.world[0], size)).normalize()
                };
                let angle = observed.y.atan2(observed.x).to_degrees().rem_euclid(180.0);
                let error = observed.dot(projected).abs().min(1.0).acos().to_degrees();
                writeln!(report, "    photo angle {angle:.2} deg; capture angle error {error:.2} deg")?;
            }
        }
        overlay(&photo, initial, &features).save(output.join(format!("{}-before.png", view.name)))?;
        overlay(&photo, fitted, &features).save(output.join(format!("{}-fit.png", view.name)))?;
        overlay(&photo, calibrated, &features).save(output.join(format!("{}-reference-fit.png", view.name)))?;
        if args.iter().any(|arg| arg == "--write") {
            if !architecture.valid(fitted, &features, size) {
                return Err(format!("{}: fit has an obstructed architectural landmark", view.name).into());
            }
            definitions = definitions
                .lines()
                .map(|line| {
                    if line.contains(&format!("View::new(\"{}\"", view.name)) {
                        format!("        {}", fitted.definition(view.name))
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            definitions.push('\n');
        }
    }
    if args.iter().any(|arg| arg == "--write") {
        fs::write(source, definitions)?;
    }
    fs::write(output.join("fit.txt"), &report)?;
    print!("{report}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_fit_recovers_pose_and_separates_reference_roll() {
        let true_camera = Camera([0.2, -0.4, 1.5, 0.7, -0.25, (0.6f64).tan().ln(), 0.04]);
        let size = DVec2::new(1448.0, 1086.0);
        let mut features = Vec::new();
        for x in [1.5, 3.0] {
            for y in [1.0, 2.0] {
                for z in [0.0, 2.4] {
                    let point = DVec3::new(x, y, z);
                    features.push(Feature {
                        label: "corner",
                        world: [point; 2],
                        photo: [true_camera.project(point, size); 2],
                        point: true,
                        axis: false,
                    });
                }
            }
        }
        let mut initial = true_camera;
        initial.0[0] += 0.3;
        initial.0[1] -= 0.2;
        initial.0[3] += 0.15;
        initial.0[4] += 0.1;
        initial.0[5] += 0.2;
        initial.0[6] = 0.0;
        let result = fit(initial, &features, size, None, true);
        assert!(loss(&residuals(result, &features, size)) / (features.len() as f64) < 1.0);
        assert!((result.eye() - true_camera.eye()).length() < 0.02);
        let mut render = result;
        render.0[6] = 0.0;
        assert!(render.basis().1.z.abs() < 1e-12);
    }
    #[test]
    fn finite_architectural_lines_recover_camera_without_extrapolating_observations() {
        let truth = Camera([0.2, -0.4, 1.5, 0.7, -0.25, 0.6f64.tan().ln(), 0.0]);
        let size = DVec2::new(1448.0, 1086.0);
        let mut features = Vec::new();
        for axis in 0..3 {
            for side in 0..4 {
                let mut start = DVec3::new(1.5, 1.0, 0.0);
                let mut end = DVec3::new(3.0, 2.5, 2.4);
                for other in 0..3 {
                    if axis != other {
                        let rank = if other < axis { other } else { other - 1 };
                        let value = if side & (1 << rank) == 0 { start[other] } else { end[other] };
                        start[other] = value;
                        end[other] = value;
                    }
                }
                features.push(Feature {
                    label: "architectural edge",
                    world: [start, end],
                    photo: [truth.project(start.lerp(end, 0.2), size), truth.project(start.lerp(end, 0.8), size)],
                    point: false,
                    axis: false,
                });
            }
        }
        let mut initial = truth;
        initial.0[0] += 0.2;
        initial.0[1] -= 0.2;
        initial.0[3] += 0.12;
        initial.0[4] -= 0.08;
        initial.0[5] += 0.15;
        let recovered = fit(initial, &features, size, None, false);
        let errors = residuals(recovered, &features, size);
        assert!(loss(&errors) / (errors.len() as f64) < 1.0);
        assert!((recovered.eye() - truth.eye()).length() < 0.03);
        assert!(recovered.0[6].abs() < 1e-12);
    }
}
