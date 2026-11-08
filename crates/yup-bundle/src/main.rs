//! `cargo run -p yup-bundle`: builds `target/yup.mjs`, one file that runs the tool anywhere
//! JavaScript has WebAssembly.
//!
//! It builds the `yup-wasm` module with the small `wasm` profile, embeds it as base64 in
//! `js/template.mjs`, with the tool's and the language's versions, and copies the types,
//! `js/yup.d.ts`, beside it.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match bundle() {
        Ok(message) => {
            eprintln!("yup-bundle: {message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("yup-bundle: {message}");
            ExitCode::FAILURE
        }
    }
}

fn bundle() -> Result<String, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the bundler is two folders inside the workspace");
    let target = env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let status = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "-p",
            "yup-wasm",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            "wasm",
        ])
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !status.success() {
        return Err("building the WebAssembly module failed".to_string());
    }
    let read =
        |path: &Path| fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()));
    let wasm = read(&target.join("wasm32-unknown-unknown/wasm/yup_wasm.wasm"))?;
    let template = String::from_utf8(read(&root.join("js/template.mjs"))?)
        .map_err(|_| "js/template.mjs is not valid UTF-8".to_string())?;
    let bundled = fill(
        &template,
        &[
            ("__YUP_WASM__", &base64(&wasm)),
            ("__YUP_VERSION__", env!("CARGO_PKG_VERSION")),
            ("__YUP_LANGUAGE__", yup_core::LANGUAGE_VERSION),
        ],
    )?;
    let write = |path: &Path, bytes: &[u8]| {
        fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
    };
    let output = target.join("yup.mjs");
    write(&output, bundled.as_bytes())?;
    write(&target.join("yup.d.ts"), &read(&root.join("js/yup.d.ts"))?)?;
    Ok(format!(
        "wrote {} ({} KB, with a module of {} KB)",
        output.display(),
        bundled.len() / 1024,
        wasm.len() / 1024
    ))
}

/// The template with each placeholder, which appears exactly once, replaced by its value.
fn fill(template: &str, values: &[(&str, &str)]) -> Result<String, String> {
    let mut filled = template.to_string();
    for (placeholder, value) in values {
        match filled.matches(placeholder).count() {
            1 => filled = filled.replacen(placeholder, value, 1),
            count => {
                return Err(format!(
                    "the template has {placeholder} {count} times, where it needs it once"
                ));
            }
        }
    }
    Ok(filled)
}

/// Base64 in the standard alphabet, padded (RFC 4648 §4), as `atob` reads it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().enumerate().fold(0_u32, |group, (i, &byte)| {
            group | u32::from(byte) << (16 - 8 * i)
        });
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (group >> (18 - 6 * i)) & 0x3f;
                out.push(char::from(ALPHABET[index as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfcs_test_vectors() {
        // RFC 4648 §10.
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in vectors {
            assert_eq!(base64(plain.as_bytes()), encoded, "{plain}");
        }
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn each_placeholder_is_filled_once() {
        assert_eq!(
            fill("a \"__X__\" b", &[("__X__", "1")]),
            Ok("a \"1\" b".to_string())
        );
        assert_eq!(
            fill("__X__ __X__", &[("__X__", "1")]),
            Err("the template has __X__ 2 times, where it needs it once".to_string())
        );
    }
}
