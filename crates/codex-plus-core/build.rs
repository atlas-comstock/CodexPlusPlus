use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.join("../..");
    let env_file = workspace_root.join(".env.freecodex.local");

    println!("cargo:rerun-if-env-changed=OPENCODE_API_KEY");
    println!("cargo:rerun-if-env-changed=OPENROUTER_API_KEY");
    println!("cargo:rerun-if-env-changed=NVIDIA_API_KEY");
    println!("cargo:rerun-if-changed={}", env_file.display());

    let mut opencode = env::var("OPENCODE_API_KEY").unwrap_or_default();
    let mut openrouter = env::var("OPENROUTER_API_KEY").unwrap_or_default();
    let mut nvidia = env::var("NVIDIA_API_KEY").unwrap_or_default();

    if let Ok(contents) = fs::read_to_string(&env_file) {
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = parse_export_line(line) else {
                continue;
            };
            match key {
                "OPENCODE_API_KEY" if opencode.is_empty() => opencode = value,
                "OPENROUTER_API_KEY" if openrouter.is_empty() => openrouter = value,
                "NVIDIA_API_KEY" if nvidia.is_empty() => nvidia = value,
                _ => {}
            }
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let generated = out_dir.join("freecodex_provider_keys_gen.rs");
    let body = format!(
        r#"pub const OPENCODE_KEY: &str = {opencode:?};
pub const OPENROUTER_KEY: &str = {openrouter:?};
pub const NVIDIA_KEY: &str = {nvidia:?};
"#
    );
    fs::write(generated, body).expect("write generated provider keys");
}

fn parse_export_line(line: &str) -> Option<(&str, String)> {
    let line = line.strip_prefix("export ")?.trim();
    let (key, rest) = line.split_once('=')?;
    let value = rest.trim().trim_matches('"').trim_matches('\'').to_string();
    Some((key.trim(), value))
}