//! Doors that open and close.
//!
//! A MAP lists each door twice: as a dynamic object (`RepeatableTouchplate`, with the
//! transform that places it, how it moves and its sounds) and as a small slab in the geometry
//! list, in the door's own space. The slab is the door leaf: a swinging door extends along
//! its local +z from the hinge at the origin, a sliding one along its local -x. Many doors
//! share one model, each with its own transform.
//!
//! How the numbers read, checked against rm11 (swinging) and rm02 (swinging and sliding):
//! - `oneTimeRotation`: the second value of `direction` is the angle the door turns, in
//!   radians, and its sign picks the way; the first is twice that, which is the turning
//!   speed that makes every door take half a second.
//! - `oneTimeTranslation`: the second value is how far the door slides and the first the
//!   speed in units a second (150 for 115 units, about 0.77 s). The last value and
//!   `velocity` make a world X/Z direction (1,0), (0,1), (-1,0) or (0,-1) that says which
//!   way along the door it goes.

use crate::map::{self, DynamicObjectKind, Map};

type V3 = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    /// Turns about the hinge (the door's local Y axis) by this angle in radians, signed
    Swing { angle: f32 },
    /// Slides along this unit direction, in the world, by this distance
    Slide { dir: V3, distance: f32 },
}

#[derive(Clone, Debug)]
pub struct Door {
    /// This door's own name, like "212_doorknobinterior03"
    pub name: String,
    /// The object in the geometry list that is the door leaf
    pub model: String,
    /// Where that object is in the geometry list
    pub model_index: usize,
    /// Where the leaf's origin is when the door is closed, and its local axes in the world
    pub origin: V3,
    pub axes: [V3; 3],
    pub motion: Motion,
    /// How long it takes to open or close
    pub seconds: f32,
    pub open_sound: String,
    pub close_sound: String,
    /// The portal in the doorway, if one lines up with the leaf, and the rooms it joins
    pub portal: Option<usize>,
    pub rooms: Vec<u32>,
    /// The middle of the leaf in the door's own space
    pub centre_local: V3,
}

impl Door {
    /// A point of the door's own space in the world, with the door `open` of the way (0 to 1)
    pub fn place(&self, p: V3, open: f32) -> V3 {
        let (mut x, y, mut z) = (p[0], p[1], p[2]);
        let mut shift = [0.0; 3];
        match self.motion {
            Motion::Swing { angle } => {
                let (s, c) = (angle * open).sin_cos();
                (x, z) = (x * c + z * s, -x * s + z * c);
            }
            Motion::Slide { dir, distance } => {
                shift = [dir[0] * distance * open, dir[1] * distance * open, dir[2] * distance * open];
            }
        }
        let [ax, ay, az] = self.axes;
        std::array::from_fn(|i| self.origin[i] + ax[i] * x + ay[i] * y + az[i] * z + shift[i])
    }

    /// The middle of the leaf in the world, with the door `open` this far
    pub fn centre(&self, open: f32) -> V3 {
        self.place(self.centre_local, open)
    }

    /// The 4x4 column-major matrix that does `place`, for OpenGL
    pub fn matrix(&self, open: f32) -> [f32; 16] {
        let o = self.place([0.0; 3], open);
        let column = |p: V3| -> [f32; 3] {
            let q = self.place(p, open);
            [q[0] - o[0], q[1] - o[1], q[2] - o[2]]
        };
        let (x, y, z) = (column([1.0, 0.0, 0.0]), column([0.0, 1.0, 0.0]), column([0.0, 0.0, 1.0]));
        [x[0], x[1], x[2], 0.0, y[0], y[1], y[2], 0.0, z[0], z[1], z[2], 0.0, o[0], o[1], o[2], 1.0]
    }
}

/// Every door in the level
pub fn doors(level: &Map) -> Vec<Door> {
    let mut out = Vec::new();
    for d in &level.dynamic_objects.dynamic_objects {
        let DynamicObjectKind::RepeatableTouchplate { animation_type, direction, name2, name3, .. } = &d.kind else { continue };
        let Some(model_index) = level.geometries.objects.iter().position(|o| o.name.eq_ignore_ascii_case(&d.name)) else { continue };
        let tm = &d.tm;
        let axes = [
            [tm.x_axis.x, tm.x_axis.y, tm.x_axis.z],
            [tm.y_axis.x, tm.y_axis.y, tm.y_axis.z],
            [tm.z_axis.x, tm.z_axis.y, tm.z_axis.z],
        ];
        let origin = [tm.position.x, tm.position.y, tm.position.z];

        let (motion, seconds) = match animation_type.as_str() {
            "oneTimeRotation" => {
                let angle = direction.y;
                let speed = direction.x.abs();
                (Motion::Swing { angle }, if speed > 1e-3 { angle.abs() / speed } else { 0.5 })
            }
            "oneTimeTranslation" => {
                // Slides along its own width, the way round given by the world X/Z pair
                let toward = [direction.z, 0.0, if let DynamicObjectKind::RepeatableTouchplate { velocity, .. } = &d.kind { *velocity } else { 0.0 }];
                let along = axes[0];
                let sign = if toward[0] * along[0] + toward[2] * along[2] < 0.0 { -1.0 } else { 1.0 };
                let distance = direction.y;
                let speed = direction.x.abs();
                (Motion::Slide { dir: [along[0] * sign, along[1] * sign, along[2] * sign], distance }, if speed > 1e-3 { distance / speed } else { 0.8 })
            }
            _ => continue,
        };

        let mut door = Door {
            name: d.section_name.clone(),
            model: d.name.clone(),
            model_index,
            origin,
            axes,
            motion,
            seconds,
            open_sound: name2.clone(),
            close_sound: name3.clone(),
            portal: None,
            rooms: map::object_room(&d.section_name).into_iter().collect(),
            centre_local: [0.0; 3],
        };

        // The portal in the doorway is the one the closed leaf covers
        door.centre_local = leaf_centre(level, &door);
        let centre = door.centre(0.0);
        let nearest = level.portals.portals.iter().enumerate()
            .map(|(i, p)| {
                let n = p.coordinates.len().max(1) as f32;
                let c = [
                    p.coordinates.iter().map(|v| v.x).sum::<f32>() / n,
                    p.coordinates.iter().map(|v| v.y).sum::<f32>() / n,
                    p.coordinates.iter().map(|v| v.z).sum::<f32>() / n,
                ];
                (i, ((c[0] - centre[0]).powi(2) + (c[1] - centre[1]).powi(2) + (c[2] - centre[2]).powi(2)).sqrt())
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, dist)) = nearest {
            if dist <= PORTAL_MATCH {
                let p = &level.portals.portals[i];
                door.portal = Some(i);
                door.rooms = vec![p.room, p.opposite_room];
            }
        }
        out.push(door);
    }
    out
}

/// How near a portal's middle must be to the closed leaf's for the door to be in it
const PORTAL_MATCH: f32 = 40.0;

/// The middle of the leaf, in the door's own space
fn leaf_centre(level: &Map, door: &Door) -> V3 {
    let object = &level.geometries.objects[door.model_index];
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &object.vertices {
        for (a, x) in [v.x, v.y, v.z].into_iter().enumerate() {
            lo[a] = lo[a].min(x);
            hi[a] = hi[a].max(x);
        }
    }
    [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0]
}

/// The door leaf as triangles in the door's own space, with the MAP material of each
pub fn leaf_triangles(level: &Map, door: &Door) -> Vec<([V3; 3], usize)> {
    let object = &level.geometries.objects[door.model_index];
    let mut out = Vec::new();
    for od in &object.object_datas {
        for &(i0, i1, i2) in &od.faces.face_indices {
            let (Some(a), Some(b), Some(c)) = (object.vertices.get(i0 as usize), object.vertices.get(i1 as usize), object.vertices.get(i2 as usize)) else { continue };
            out.push(([[a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z]], od.mn as usize));
        }
    }
    out
}

/// How open a door is, and which way it is going
#[derive(Clone, Copy, Debug, Default)]
pub struct DoorState {
    /// 0 closed to 1 open
    pub open: f32,
    /// Where it is heading: true to open, false to close
    pub opening: bool,
}

impl DoorState {
    /// Move `dt` seconds toward the target; true if it moved
    pub fn advance(&mut self, door: &Door, dt: f32) -> bool {
        let target = if self.opening { 1.0 } else { 0.0 };
        if self.open == target {
            return false;
        }
        let step = dt / door.seconds.max(0.05);
        self.open = if self.opening { (self.open + step).min(1.0) } else { (self.open - step).max(0.0) };
        true
    }

    /// Fully closed: nothing can pass or be seen through
    pub fn closed(&self) -> bool {
        self.open <= 0.0 && !self.opening
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn load(path: &str) -> Option<Map> {
        let path = Path::new(path);
        if !path.exists() {
            eprintln!("skipping: {} not found", path.display());
            return None;
        }
        Some(map::read(path).expect("map loads"))
    }

    #[test]
    fn a_swinging_door_turns_about_its_hinge() {
        let door = Door {
            name: "d".into(), model: "d".into(), model_index: 0,
            origin: [100.0, 0.0, 200.0],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            motion: Motion::Swing { angle: std::f32::consts::FRAC_PI_2 },
            seconds: 0.5, open_sound: String::new(), close_sound: String::new(), portal: None, rooms: vec![], centre_local: [0.0; 3],
        };
        // Closed, the free end (120 along z) is where it says
        assert_eq!(door.place([0.0, 0.0, 120.0], 0.0), [100.0, 0.0, 320.0]);
        // Open a quarter turn, it points along the x axis instead; the hinge stays put
        let end = door.place([0.0, 0.0, 120.0], 1.0);
        assert!((end[0] - 220.0).abs() < 0.01 && (end[2] - 200.0).abs() < 0.01, "{end:?}");
        assert_eq!(door.place([0.0, 50.0, 0.0], 1.0), [100.0, 50.0, 200.0]);
        // The matrix does the same as place
        let m = door.matrix(1.0);
        let by_matrix = [m[8] * 120.0 + m[12], m[9] * 120.0 + m[13], m[10] * 120.0 + m[14]];
        assert!((by_matrix[0] - end[0]).abs() < 0.01 && (by_matrix[2] - end[2]).abs() < 0.01);
    }

    #[test]
    fn a_door_opens_in_its_time() {
        let door = Door {
            name: "d".into(), model: "d".into(), model_index: 0, origin: [0.0; 3],
            axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            motion: Motion::Swing { angle: 1.0 }, seconds: 0.5,
            open_sound: String::new(), close_sound: String::new(), portal: None, rooms: vec![], centre_local: [0.0; 3],
        };
        let mut s = DoorState { open: 0.0, opening: true };
        assert!(s.advance(&door, 0.25) && (s.open - 0.5).abs() < 1e-6);
        assert!(s.advance(&door, 1.0) && s.open == 1.0);
        assert!(!s.advance(&door, 1.0));
        s.opening = false;
        assert!(!s.closed());
        while s.advance(&door, 0.1) {}
        assert!(s.closed());
    }

    #[test]
    fn rm11_doors_are_swinging_doors_in_their_doorways() {
        let Some(map) = load("data/map/rm11/rm11.map") else { return };
        let doors = doors(&map);
        assert!(doors.len() >= 50, "{} doors", doors.len());
        assert!(doors.iter().all(|d| matches!(d.motion, Motion::Swing { .. })));
        // The angles are quarter turns, and they take half a second
        assert!(doors.iter().all(|d| match d.motion {
            Motion::Swing { angle } => (angle.abs() - 1.57).abs() < 0.01,
            _ => false,
        }));
        assert!(doors.iter().all(|d| (d.seconds - 0.5).abs() < 0.01));
        // Nearly every door lines up with a portal
        let lined_up = doors.iter().filter(|d| d.portal.is_some()).count();
        assert!(lined_up * 10 >= doors.len() * 9, "{lined_up} of {} doors have a portal", doors.len());
    }

    #[test]
    fn rm02_has_sliding_doors_that_slide_along_their_width() {
        let Some(map) = load("data/map/rm02/rm02.map") else { return };
        let doors = doors(&map);
        let slides: Vec<_> = doors.iter().filter(|d| matches!(d.motion, Motion::Slide { .. })).collect();
        assert!(!slides.is_empty() && slides.len() < doors.len(), "{} slides of {}", slides.len(), doors.len());
        for d in slides {
            let Motion::Slide { dir, distance } = d.motion else { unreachable!() };
            assert!((distance - 115.0).abs() < 0.01);
            // Along the leaf's own width, so a wall can hold it
            let along = d.axes[0];
            assert!((dir[0] * along[0] + dir[1] * along[1] + dir[2] * along[2]).abs() > 0.99);
            // Open, the leaf has left the doorway by its distance
            let (closed, open) = (d.place([0.0; 3], 0.0), d.place([0.0; 3], 1.0));
            let moved = ((open[0] - closed[0]).powi(2) + (open[2] - closed[2]).powi(2)).sqrt();
            assert!((moved - 115.0).abs() < 0.01);
        }
    }
}
