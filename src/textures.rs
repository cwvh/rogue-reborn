//! Finding texture files in the game data and loading them.

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use rogue_reborn::{map, rsb, tga};
use crate::opengl::*;

// Load the texture for a material name such as "art_h_torso.bmp" from a
// character directory: the .rsb of the same name, or for .tga materials the
// .tga itself. File names differ in case from the references.
pub fn load_character_texture(dir: &Path, material: &str) -> Option<u32> {
    let want = Path::new(material);
    let stem = want.file_stem()?.to_string_lossy().to_lowercase();
    let is_tga = want.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga"));
    let path = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        let ext = if is_tga { "tga" } else { "rsb" };
        p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
            && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem)
    })?;
    if is_tga {
        let t = tga::read(&path).ok()?;
        upload_rgba_to_gl(t.width, t.height, &t.rgba)
    } else {
        upload_rsb_to_gl(&rsb::read(&path).ok()?)
    }
}

pub fn ensure_texture(
    materials: &map::Materials,
    mat_index: usize,
    map_num: &str,
    cache: &mut HashMap<String, Option<u32>>,
) -> Option<u32> {
    if mat_index >= materials.materials.len() {
        return None;
    }
    let filename = &materials.materials[mat_index].filename;
    if let Some(id) = cache.get(filename) {
        return *id;
    }

    // A texture that is missing or unreadable is remembered too: looking for it lists two
    // directories, and doing that for every face group every frame took 30 ms on rm12
    let tex_id = resolve_texture_path(map_num, filename)
        .and_then(|resolved| rsb::read(&resolved).ok())
        .and_then(|rsb_tex| upload_rsb_to_gl(&rsb_tex));
    cache.insert(filename.clone(), tex_id);
    tex_id
}

// Find an on-disk RSB path by searching map-local and shared texture dirs,
// converting MAP's .bmp references to .RSB/.rsb.
pub fn resolve_texture_path(map_num: &str, material_filename: &str) -> Option<PathBuf> {
    // Preserve any subdirectories from the material filename (e.g., "faces/foo.bmp")
    let rel = Path::new(material_filename);

    // Texture name without extension (MAP refers to .bmp)
    let stem = rel.file_stem()?.to_string_lossy().into_owned();


    // File names differ in case from the MAP's (the data comes from a
    // case-insensitive filesystem), and .tga materials are stored as
    // "TGA<name>.rsb"
    let mut wanted = vec![format!("{stem}.rsb").to_lowercase()];
    if rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga")) {
        wanted.push(format!("tga{stem}.rsb").to_lowercase());
    }
    let parent = rel.parent().unwrap_or(Path::new(""));

    for dir in [Path::new("data").join("map").join(map_num).join(parent), Path::new("data").join("texture").join(parent)] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let names: Vec<(String, PathBuf)> = entries.flatten()
            .map(|e| (e.file_name().to_string_lossy().to_lowercase(), e.path())).collect();
        for want in &wanted {
            if let Some((_, path)) = names.iter().find(|(n, _)| n == want) {
                return Some(path.clone());
            }
        }
    }

    None
}
