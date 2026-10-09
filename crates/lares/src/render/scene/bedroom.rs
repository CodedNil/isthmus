use super::{Material, Sample};
use isthmus::prelude::*;

#[inline(never)]
pub(super) fn bed<S: Sample>(local: Vec3, size: Vec3) -> S {
    let base = super::scaled_box(local, size, vec3(0.0, 0.0, -0.32), vec3(1.0, 1.0, 0.36), 0.03);
    let mattress = super::scaled_box(local, size, vec3(0.0, 0.0, 0.02), vec3(0.94, 0.96, 0.44), 0.12);
    let pillow = super::scaled_box(local, size, vec3(0.0, -0.36, 0.1), vec3(0.7, 0.2, 0.2), 0.2);
    let head = super::scaled_box(local, size, vec3(0.0, -0.46, 0.12), vec3(1.0, 0.06, 0.72), 0.06);
    let distance = super::blend(super::blend(base, mattress, 0.06), pillow, 0.05).min_num(head);
    let (material, tint) = if pillow < base.min_num(mattress).min_num(head) {
        (Material::Fabric, vec3(0.78, 0.82, 0.9))
    } else if base.min_num(head) < mattress {
        (Material::Wood, Vec3::ONE)
    } else {
        (Material::Fabric, vec3(0.68, 0.62, 0.54))
    };
    S::new(distance, || (material, tint))
}
