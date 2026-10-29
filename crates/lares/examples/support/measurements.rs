//! Manually measured architectural correspondences in the original 1448×1086
//! photographs. World coordinates use the renderer's metre/Z-up convention.
//! Furniture, reflected edges and open door leaves are deliberately excluded.
use isthmus::glam::{DVec2, DVec3};

pub struct Feature {
    pub label: &'static str,
    pub world: [DVec3; 2],
    pub photo: [DVec2; 2],
    pub point: bool,
    pub axis: bool,
}
/// A photographed patch of wall must be directly visible. This constrains
/// foreground occlusion without using any furniture placement as a landmark.
pub struct VisibleWall {
    pub normal: DVec3,
    pub point: DVec3,
    pub pixel: DVec2,
}

fn point(label: &'static str, world: [f64; 3], photo: [f64; 2]) -> Feature {
    Feature { label, world: [DVec3::from(world); 2], photo: [DVec2::from(photo); 2], point: true, axis: false }
}
fn line(label: &'static str, world: [[f64; 3]; 2], photo: [[f64; 2]; 2]) -> Feature {
    Feature { label, world: world.map(DVec3::from), photo: photo.map(DVec2::from), point: false, axis: false }
}
fn axis(label: &'static str, direction: [f64; 3], photo: [[f64; 2]; 2]) -> Feature {
    Feature { label, world: [DVec3::from(direction); 2], photo: photo.map(DVec2::from), point: false, axis: true }
}
pub fn for_view(name: &str) -> Vec<Feature> {
    match name {
        "entrance" => vec![
            point("far wall floor corner", [0.15, -2.85, 0.0], [400.0, 792.0]),
            line("door head", [[-0.75, -2.85, 2.05], [0.05, -2.85, 2.05]], [[0.0, 1.0], [326.0, 54.0]]),
            line("door threshold", [[-0.75, -2.85, 0.0], [0.05, -2.85, 0.0]], [[0.0, 924.0], [400.0, 792.0]]),
            line("far vertical wall corner", [[0.15, -2.85, 0.0], [0.15, -2.85, 2.4]], [[327.0, 0.0], [400.0, 792.0]]),
            line("near vertical wall corner", [[0.15, -0.85, 0.0], [0.15, -0.85, 2.4]], [[1098.0, 0.0], [
                1007.0, 1086.0,
            ]]),
            line("visible skirting base", [[0.15, -2.85, 0.0], [0.15, -2.70, 0.0]], [[400.0, 792.0], [430.0, 813.0]]),
        ],
        "kitchen-window" => vec![
            point("opening upper left", [-4.60, -2.85, 2.05], [632.0, 244.0]),
            point("opening upper right", [-3.20, -2.85, 2.05], [1062.0, 244.0]),
            point("opening lower left", [-4.60, -2.85, 1.02], [632.0, 585.0]),
            point("opening lower right", [-3.20, -2.85, 1.02], [1062.0, 584.0]),
            point("left ceiling corner", [-5.75, -2.85, 2.4], [233.0, 157.0]),
            line("back ceiling junction", [[-5.75, -2.85, 2.4], [-2.45, -2.85, 2.4]], [[233.0, 157.0], [
                1396.0, 157.0,
            ]]),
            axis("left ceiling depth", [0.0, 1.0, 0.0], [[233.0, 157.0], [0.0, 32.0]]),
        ],
        "kitchen-cooker" => vec![
            point("opening upper left", [-4.60, -2.85, 2.05], [1315.0, 194.0]),
            point("opening lower left", [-4.60, -2.85, 1.02], [1273.0, 546.0]),
            point("ceiling corner", [-5.75, -2.85, 2.4], [972.0, 173.0]),
            line("back ceiling junction", [[-5.75, -2.85, 2.4], [-4.60, -2.85, 2.4]], [[972.0, 173.0], [1448.0, 72.0]]),
            axis("side ceiling junction", [0.0, 1.0, 0.0], [[245.0, 0.0], [972.0, 173.0]]),
            axis("wall corner vertical", [0.0, 0.0, 1.0], [[972.0, 173.0], [961.0, 566.0]]),
        ],
        "kitchen-fridge" => vec![
            axis("window reveal vertical", [0.0, 0.0, 1.0], [[283.0, 208.0], [316.0, 569.0]]),
            axis("back ceiling junction", [1.0, 0.0, 0.0], [[0.0, 49.0], [566.0, 167.0]]),
            axis("side ceiling junction", [0.0, 1.0, 0.0], [[566.0, 167.0], [1100.0, 0.0]]),
        ],
        // This close crop lacks a second metric corner. Fit orientation/FOV
        // from architectural directions while retaining the existing position.
        "kitchen-cabinet" => vec![
            axis("window reveal vertical", [0.0, 0.0, 1.0], [[336.0, 331.0], [252.0, 1086.0]]),
            axis("window head", [1.0, 0.0, 0.0], [[0.0, 258.0], [336.0, 331.0]]),
            axis("ceiling depth", [0.0, 1.0, 0.0], [[1072.0, 213.0], [1448.0, 37.0]]),
        ],
        "bookshelf" => vec![
            point("pier right floor corner", [-0.85, -0.65, 0.0], [1108.0, 1021.0]),
            line("pier left edge", [[-2.45, -0.65, 0.0], [-2.45, -0.65, 2.4]], [[171.0, 0.0], [306.0, 714.0]]),
            line("pier right edge", [[-0.85, -0.65, 0.0], [-0.85, -0.65, 2.4]], [[1162.0, 0.0], [1108.0, 1021.0]]),
            line("pier skirting base", [[-1.25, -0.65, 0.0], [-0.85, -0.65, 0.0]], [[850.0, 982.0], [1108.0, 1021.0]]),
            line("entrance opening head", [[-0.75, -2.85, 2.05], [0.05, -2.85, 2.05]], [[1172.0, 79.0], [
                1448.0, 63.0,
            ]]),
            line("entrance threshold", [[-0.75, -2.85, 0.0], [0.05, -2.85, 0.0]], [[1175.0, 638.0], [1431.0, 653.0]]),
        ],
        "lounge" => vec![
            point("opening sill corner", [-4.60, 2.85, 0.9], [304.0, 391.0]),
            line("window side reveal", [[-4.60, 2.85, 0.9], [-4.60, 2.85, 2.05]], [[230.0, 0.0], [304.0, 391.0]]),
            line("back wall corner", [[-5.75, 2.85, 0.9], [-5.75, 2.85, 2.4]], [[525.0, 0.0], [534.0, 160.0]]),
            line("window sill", [[-4.60, 2.85, 0.9], [-3.20, 2.85, 0.9]], [[0.0, 449.0], [304.0, 391.0]]),
        ],
        "desk" => vec![
            point("opening sill corner", [-0.65, 2.85, 0.9], [1113.0, 449.0]),
            line("back wall corner", [[0.25, 2.85, 0.0], [0.25, 2.85, 2.4]], [[881.0, 0.0], [865.0, 232.0]]),
            line("side skirting base", [[0.25, 0.35, 0.0], [0.25, 1.70, 0.0]], [[40.0, 922.0], [525.0, 779.0]]),
            line("window wall skirting", [[-0.65, 2.85, 0.0], [-1.15, 2.85, 0.0]], [[1303.0, 822.0], [1448.0, 874.0]]),
            axis("window reveal vertical", [0.0, 0.0, 1.0], [[1190.0, 0.0], [1113.0, 449.0]]),
        ],
        "dining" => vec![
            point("desk opening upper jamb", [-1.35, 2.85, 2.05], [484.0, 172.0]),
            point("desk opening lower jamb", [-1.35, 2.85, 0.9], [496.0, 548.0]),
            point("lounge opening upper jamb", [-3.20, 2.85, 2.05], [1147.0, 143.0]),
            point("lounge opening lower jamb", [-3.20, 2.85, 0.9], [1121.0, 563.0]),
            line("wall ceiling junction", [[-1.0, 2.85, 2.4], [-4.0, 2.85, 2.4]], [[0.0, 86.0], [1448.0, 9.0]]),
            line("wall skirting base", [[-1.0, 2.85, 0.0], [-3.0, 2.85, 0.0]], [[300.0, 807.0], [1108.0, 837.0]]),
        ],
        "bedroom1" => vec![
            point("window wall ceiling corner", [5.75, 2.85, 2.4], [1251.0, 81.0]),
            axis("window wall vertical", [0.0, 0.0, 1.0], [[1251.0, 81.0], [1238.0, 426.0]]),
            axis("back ceiling junction", [0.0, 1.0, 0.0], [[550.0, 175.0], [1251.0, 81.0]]),
            point("visible window sill reveal", [4.45, 2.85, 0.9], [1357.0, 615.0]),
            axis("window sill", [1.0, 0.0, 0.0], [[1357.0, 615.0], [1448.0, 644.0]]),
        ],
        "bedroom2" => vec![
            point("back ceiling corner", [5.75, 2.85, 2.4], [769.0, 130.0]),
            point("window sill centre", [3.85, 2.85, 0.9], [1187.0, 447.0]),
            line("wall vertical corner", [[5.75, 2.85, 0.0], [5.75, 2.85, 2.4]], [[769.0, 130.0], [773.0, 367.0]]),
            axis("head wall ceiling junction", [0.0, 1.0, 0.0], [[462.0, 0.0], [769.0, 130.0]]),
            axis("window wall ceiling junction", [1.0, 0.0, 0.0], [[769.0, 130.0], [1448.0, 0.0]]),
            line("window wall skirting base", [[3.0, 2.85, 0.0], [4.0, 2.85, 0.0]], [[1279.0, 656.0], [1448.0, 692.0]]),
        ],
        "bedroom3" => vec![
            point("ensuite opening upper left", [1.95, 2.8, 2.05], [506.0, 239.0]),
            point("ensuite opening upper right", [1.95, 2.0, 2.05], [651.0, 230.0]),
            point("ensuite opening lower left", [1.95, 2.8, 0.0], [529.0, 657.0]),
            point("ceiling corner", [1.95, 2.85, 2.4], [444.0, 164.0]),
            axis("window wall ceiling junction", [1.0, 0.0, 0.0], [[0.0, 63.0], [444.0, 164.0]]),
            axis("dresser wall ceiling junction", [0.0, 1.0, 0.0], [[444.0, 164.0], [1225.0, 0.0]]),
        ],
        "bathroom1" => vec![
            line("bath wall ceiling junction", [[0.25, -1.8, 2.4], [0.25, -2.6, 2.4]], [[248.0, 0.0], [610.0, 143.0]]),
            line("back wall ceiling junction", [[1.5, -2.85, 2.4], [2.2, -2.85, 2.4]], [[746.0, 126.0], [939.0, 43.0]]),
            axis("bath wall tile cap", [0.0, 1.0, 0.0], [[120.0, 497.0], [628.0, 440.0]]),
            axis("back wall tile cap", [1.0, 0.0, 0.0], [[746.0, 441.0], [915.0, 455.0]]),
            axis("back wall tile vertical", [0.0, 0.0, 1.0], [[828.0, 450.0], [817.0, 575.0]]),
            axis("bath wall tile vertical", [0.0, 0.0, 1.0], [[363.0, 467.0], [388.0, 640.0]]),
        ],
        "bathroom2" => vec![
            point("door jamb floor", [2.5, -0.95, 0.0], [1251.0, 1059.0]),
            axis("back wall tile cap", [1.0, 0.0, 0.0], [[0.0, 593.0], [355.0, 526.0]]),
            axis("radiator wall tile cap", [0.0, 1.0, 0.0], [[942.0, 535.0], [1369.0, 597.0]]),
            line("radiator wall skirting base", [[2.526, -2.3, 0.0], [2.526, -1.1, 0.0]], [[902.0, 939.0], [
                1253.0, 1051.0,
            ]]),
            axis("back wall tile vertical", [0.0, 0.0, 1.0], [[375.0, 549.0], [391.0, 704.0]]),
            axis("radiator wall tile vertical", [0.0, 0.0, 1.0], [[1108.0, 568.0], [1060.0, 789.0]]),
        ],
        "ensuite1" => vec![
            point("toilet wall floor corner", [0.374, 2.826, 0.0], [520.0, 854.0]),
            point("recess floor corner", [1.05, 1.25, 0.0], [1014.0, 925.0]),
            line("toilet wall vertical corner", [[0.35, 2.85, 0.0], [0.35, 2.85, 2.4]], [[338.0, 0.0], [520.0, 854.0]]),
            line("recess vertical corner", [[1.05, 1.25, 0.0], [1.05, 1.25, 2.4]], [[1106.0, 0.0], [1014.0, 925.0]]),
            axis("toilet wall tile cap", [0.0, 1.0, 0.0], [[439.0, 429.0], [727.0, 400.0]]),
            axis("window wall tile cap", [1.0, 0.0, 0.0], [[251.0, 469.0], [430.0, 450.0]]),
        ],
        "ensuite2" => vec![
            point("bedroom opening upper right", [1.85, 2.8, 2.05], [675.0, 58.0]),
            point("bedroom opening lower right", [1.85, 2.8, 0.0], [735.0, 795.0]),
            point("bedroom opening lower left", [1.85, 2.0, 0.0], [509.0, 963.0]),
            line("bedroom opening head", [[1.85, 2.0, 2.05], [1.85, 2.8, 2.05]], [[235.0, 0.0], [675.0, 58.0]]),
            axis("bedroom jamb vertical", [0.0, 0.0, 1.0], [[235.0, 0.0], [509.0, 963.0]]),
            axis("window sill", [1.0, 0.0, 0.0], [[930.0, 578.0], [1448.0, 708.0]]),
        ],
        "boiler" => vec![
            point("opening upper left", [1.8, 0.15, 2.05], [557.0, 60.0]),
            point("opening lower left", [1.8, 0.15, 0.0], [628.0, 946.0]),
            point("opening lower right", [1.2, 0.15, 0.0], [876.0, 1086.0]),
            line("opening right jamb", [[1.2, 0.15, 0.0], [1.2, 0.15, 2.05]], [[1035.0, 0.0], [876.0, 1086.0]]),
            axis("opening head", [1.0, 0.0, 0.0], [[554.0, 58.0], [884.0, 0.0]]),
            axis("opening threshold", [1.0, 0.0, 0.0], [[628.0, 946.0], [876.0, 1086.0]]),
        ],
        "spare" => vec![
            // The nominal opening width is uncertain. Its centre and height,
            // plus the room floor corner, avoid fitting an assumed width.
            point("opening upper centre", [5.75, -1.8, 2.05], [1032.5, 222.0]),
            point("opening lower centre", [5.75, -1.8, 0.9], [992.5, 523.0]),
            point("floor corner", [5.75, -0.95, 0.0], [1313.0, 825.0]),
            line("window wall skirting base", [[5.75, -2.5, 0.0], [5.75, -0.95, 0.0]], [[876.0, 681.0], [
                1313.0, 825.0,
            ]]),
            axis("window wall ceiling junction", [0.0, 1.0, 0.0], [[896.0, 180.0], [1448.0, 40.0]]),
            axis("opening left vertical", [0.0, 0.0, 1.0], [[909.0, 245.0], [879.0, 511.0]]),
            axis("opening right vertical", [0.0, 0.0, 1.0], [[1156.0, 199.0], [1106.0, 535.0]]),
        ],
        _ => vec![],
    }
}

pub fn visible_walls(name: &str) -> Vec<VisibleWall> {
    match name {
        "ensuite1" => {
            vec![VisibleWall { normal: DVec3::Y, point: DVec3::new(1.5, 1.25, 1.3), pixel: DVec2::new(1420.0, 460.0) }]
        }
        _ => vec![],
    }
}
