use std::process::Command;

fn git_output(manifest_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(manifest_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PHOTOFRAME_FIRMWARE_VERSION");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    if let Some(git_dir) = git_output(&manifest_dir, &["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={manifest_dir}/{git_dir}/HEAD");
        if let Some(head_ref) = git_output(&manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={manifest_dir}/{git_dir}/{head_ref}");
        }
    }

    if let Ok(explicit) = std::env::var("PHOTOFRAME_FIRMWARE_VERSION") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            println!("cargo:rustc-env=PHOTOFRAME_FIRMWARE_VERSION={trimmed}");
            return;
        }
    }

    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let mut version = base;
    if let Some(short_sha) = git_output(&manifest_dir, &["rev-parse", "--short=8", "HEAD"]) {
        version.push('+');
        version.push_str(&short_sha);
        let dirty = Command::new("git")
            .arg("-C")
            .arg(&manifest_dir)
            .args(["diff", "--quiet", "--exit-code"])
            .status()
            .map(|status| !status.success())
            .unwrap_or(false);
        if dirty {
            version.push_str("-dirty");
        }
    }

    println!("cargo:rustc-env=PHOTOFRAME_FIRMWARE_VERSION={version}");
}
