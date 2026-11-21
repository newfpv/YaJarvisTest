use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub fn embed_ui(app: &str, override_var: &str, compress: Option<fn(&[u8]) -> Vec<u8>>) {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let ui_dir = manifest_dir.join(
        env::var_os(override_var)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("../../build/ui/{app}"))),
    );
    println!("cargo:rerun-if-changed=../embed_ui.rs");
    println!("cargo:rerun-if-env-changed={override_var}");
    println!("cargo:rerun-if-changed={}", ui_dir.display());
    assert!(
        ui_dir.join("index.html").is_file(),
        "UI not found in {}. Run npm ci and npm run build:{app} from the repository root.",
        ui_dir.display()
    );

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut assets = Vec::new();
    collect_files(&ui_dir, &mut assets);
    assets.sort();

    let mut table = String::from("pub static UI: &[(&str, &[u8])] = &[\n");
    for path in assets {
        let name = path
            .strip_prefix(&ui_dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let embedded = if let Some(compress) = compress.filter(|_| should_gzip(&name)) {
            let raw = fs::read(&path).unwrap();
            let gz = compress(&raw);
            if gz.len() < raw.len() {
                let dst = out_dir.join("ui").join(format!("{name}.gz"));
                fs::create_dir_all(dst.parent().unwrap()).unwrap();
                fs::write(&dst, gz).unwrap();
                dst
            } else {
                path
            }
        } else {
            path
        };
        table.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            embedded.to_string_lossy().replace('\\', "/")
        ));
    }
    table.push_str("];\n");
    fs::write(out_dir.join("ui_files.rs"), table).unwrap();
}

fn should_gzip(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("");
    !matches!(
        ext.to_ascii_lowercase().as_str(),
        "woff2" | "png" | "jpg" | "jpeg" | "webp" | "ico"
    )
}

fn collect_files(dir: &Path, assets: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            collect_files(&entry.path(), assets);
        } else if kind.is_file() {
            assets.push(entry.path());
        }
    }
}
