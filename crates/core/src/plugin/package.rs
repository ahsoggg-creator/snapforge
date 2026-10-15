//! `.sfplugin` archives: a zip with `plugin.toml`, the main script and a few small data files. Unpacking is strict
//! (no absolute paths or `..`, only known file types, size and count limits) so a malicious archive cannot write
//! outside the plugin folder or exhaust the disk.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use super::manifest::Manifest;

pub const EXT: &str = "sfplugin";
const MAX_ENTRIES: usize = 64;
const MAX_ENTRY_BYTES: u64 = 4 << 20;
const MAX_TOTAL_BYTES: u64 = 8 << 20;
const ALLOWED: [&str; 8] = ["toml", "rhai", "md", "txt", "png", "jpg", "jpeg", "json"];
/// Per-installation state, never part of an archive.
const LOCAL_FILES: [&str; 1] = ["state.json"];

/// A safe relative path for an archive entry, or `None`.
pub fn safe_entry_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(':') || name.starts_with('/') || name.starts_with('\\') {
        return None;
    }
    let p = PathBuf::from(name.replace('\\', "/"));
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if s.starts_with('.') || s.ends_with('.') || s.ends_with(' ') {
                    return None;
                }
                out.push(&*s);
            }
            _ => return None,
        }
    }
    let ext = out.extension()?.to_string_lossy().to_ascii_lowercase();
    ALLOWED.contains(&ext.as_str()).then_some(out)
}

fn collect(dir: &Path, base: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let path = e.path();
        let rel = path.strip_prefix(base).map_err(|e| e.to_string())?.to_path_buf();
        if path.is_dir() {
            collect(&path, base, out)?;
        } else if !LOCAL_FILES.contains(&rel.to_string_lossy().as_ref())
            && safe_entry_path(&rel.to_string_lossy()).is_some()
        {
            out.push(rel);
        }
    }
    Ok(())
}

/// Packs the plugin folder `dir` into `out`.
pub fn pack(dir: &Path, out: &Path) -> Result<(), String> {
    let manifest =
        Manifest::parse(&std::fs::read_to_string(dir.join("plugin.toml")).map_err(|e| format!("plugin.toml: {e}"))?)
            .map_err(|e| e.to_string())?;
    if !dir.join(&manifest.main).is_file() {
        return Err(format!("{} is missing", manifest.main));
    }
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    if files.len() > MAX_ENTRIES {
        return Err(format!("more than {MAX_ENTRIES} files"));
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut zip = zip::ZipWriter::new(std::fs::File::create(out).map_err(|e| e.to_string())?);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for rel in files {
        let bytes = std::fs::read(dir.join(&rel)).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_ENTRY_BYTES {
            return Err(format!("{} is larger than {} MB", rel.display(), MAX_ENTRY_BYTES >> 20));
        }
        zip.start_file(rel.to_string_lossy().replace('\\', "/"), opts)
            .map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn entry_name(f: &zip::read::ZipFile<'_, std::fs::File>) -> String {
    f.name().map(|n| n.into_owned()).unwrap_or_default()
}

fn read_entry(f: &mut zip::read::ZipFile<'_, std::fs::File>) -> Result<Vec<u8>, String> {
    if f.size() > MAX_ENTRY_BYTES {
        return Err(format!(
            "'{}' is larger than {} MB",
            entry_name(f),
            MAX_ENTRY_BYTES >> 20
        ));
    }
    let mut buf = Vec::new();
    // the declared size can lie: read at most one byte over the limit and check
    f.take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_ENTRY_BYTES {
        return Err(format!("an entry expands to more than {} MB", MAX_ENTRY_BYTES >> 20));
    }
    Ok(buf)
}

/// Reads and validates `plugin.toml` of an archive without extracting anything.
pub fn read_manifest(archive: &Path) -> Result<Manifest, String> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| e.to_string())?)
        .map_err(|e| format!("not a plugin archive: {e}"))?;
    let mut f = z
        .by_name("plugin.toml")
        .map_err(|_| "the archive has no plugin.toml".to_owned())?;
    let text = String::from_utf8(read_entry(&mut f)?).map_err(|_| "plugin.toml is not UTF-8".to_owned())?;
    Manifest::parse(&text).map_err(|e| e.to_string())
}

/// Extracts the archive into `dest` (must not exist yet). On any problem nothing is left behind.
pub fn unpack(archive: &Path, dest: &Path) -> Result<Manifest, String> {
    let manifest = read_manifest(archive)?;
    let mut z =
        zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if z.len() > MAX_ENTRIES {
        return Err(format!("the archive has more than {MAX_ENTRIES} files"));
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut total = 0u64;
        let mut has_main = false;
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| e.to_string())?;
            if f.is_dir() {
                continue;
            }
            let name = entry_name(&f);
            let rel = safe_entry_path(&name).ok_or_else(|| format!("'{name}' is not an allowed file name or type"))?;
            let bytes = read_entry(&mut f)?;
            total += bytes.len() as u64;
            if total > MAX_TOTAL_BYTES {
                return Err(format!("the archive expands to more than {} MB", MAX_TOTAL_BYTES >> 20));
            }
            has_main |= rel == Path::new(&manifest.main);
            let target = dest.join(&rel);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
        }
        if has_main {
            Ok(())
        } else {
            Err(format!("the archive has no {}", manifest.main))
        }
    })();
    match result {
        Ok(()) => Ok(manifest),
        Err(e) => {
            let _ = std::fs::remove_dir_all(dest);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sf-pkg-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample_dir(root: &Path) -> PathBuf {
        let d = root.join("src");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("plugin.toml"), "name = \"Pkg\"\nversion = \"1.0\"\n").unwrap();
        std::fs::write(d.join("main.rhai"), "log(\"hi\");\n").unwrap();
        std::fs::write(d.join("state.json"), "{\"enabled\":true}").unwrap();
        d
    }

    fn evil_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn pack_and_unpack_roundtrip_skips_local_state() {
        let root = tmp("rt");
        let src = sample_dir(&root);
        let archive = root.join("pkg.sfplugin");
        pack(&src, &archive).unwrap();
        assert_eq!(read_manifest(&archive).unwrap().name, "Pkg");
        let dest = root.join("out");
        let m = unpack(&archive, &dest).unwrap();
        assert_eq!(m.version, "1.0");
        assert_eq!(
            std::fs::read_to_string(dest.join("main.rhai")).unwrap(),
            "log(\"hi\");\n"
        );
        assert!(
            !dest.join("state.json").exists(),
            "per-installation state is never exported"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn entry_names_are_checked() {
        for ok in ["main.rhai", "lib/util.rhai", "icon.png", "docs\\readme.md"] {
            assert!(safe_entry_path(ok).is_some(), "{ok}");
        }
        for bad in [
            "../evil.rhai",
            "a/../../evil.rhai",
            "/abs.rhai",
            "\\abs.rhai",
            "C:\\x.rhai",
            "run.exe",
            "lib.dll",
            "noext",
            ".hidden.rhai",
            "a/.git/x.rhai",
            "x.rhai.",
            "x.rhai ",
            "",
        ] {
            assert!(safe_entry_path(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn zip_slip_and_executables_are_refused_and_leave_nothing() {
        let root = tmp("slip");
        let manifest: &[u8] = b"name = \"Evil\"\nversion = \"1.0\"\n";
        for (i, bad) in ["../escape.rhai", "payload.exe", "lib/../../x.rhai"].iter().enumerate() {
            let archive = root.join(format!("e{i}.sfplugin"));
            evil_zip(&archive, &[("plugin.toml", manifest), ("main.rhai", b"x"), (bad, b"x")]);
            let dest = root.join(format!("dest{i}"));
            assert!(unpack(&archive, &dest).is_err(), "{bad}");
            assert!(!dest.exists(), "the half-extracted folder is removed");
        }
        assert!(!root.join("escape.rhai").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn archives_without_a_manifest_or_main_script_are_refused() {
        let root = tmp("bad");
        let a = root.join("a.sfplugin");
        evil_zip(&a, &[("main.rhai", b"x")]);
        assert!(read_manifest(&a).unwrap_err().contains("plugin.toml"));
        let b = root.join("b.sfplugin");
        evil_zip(&b, &[("plugin.toml", b"name = \"X\"\nversion = \"1.0\"\n")]);
        assert!(unpack(&b, &root.join("dest")).unwrap_err().contains("main.rhai"));
        std::fs::write(root.join("c.sfplugin"), b"definitely not a zip").unwrap();
        assert!(read_manifest(&root.join("c.sfplugin")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn oversized_entries_are_refused() {
        let root = tmp("big");
        let a = root.join("a.sfplugin");
        let big = vec![b'a'; (MAX_ENTRY_BYTES + 10) as usize];
        evil_zip(
            &a,
            &[
                ("plugin.toml", b"name = \"X\"\nversion = \"1.0\"\n"),
                ("main.rhai", &big),
            ],
        );
        assert!(unpack(&a, &root.join("dest")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
