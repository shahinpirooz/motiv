#[path = "build/ffmpeg.rs"]
mod ffmpeg;
#[path = "build/onnxruntime.rs"]
mod onnxruntime;

fn main() {
    // Motiv: built-in OAuth app registrations
    embed_oauth_clients();

    // GPU Acceleration Detection and Build Guidance
    detect_and_report_gpu_capabilities();

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=Foundation");

        // Let the enhanced_macos crate handle its own Swift compilation
        // The swift-rs crate build will be handled in the enhanced_macos crate's build.rs
    }

    // Download and bundle FFmpeg binary at build-time to eliminate runtime download delays
    ffmpeg::ensure_ffmpeg_binary();
    onnxruntime::ensure_onnxruntime_runtime();

    tauri_build::build()
}

/// Detects GPU acceleration capabilities and provides build guidance
fn detect_and_report_gpu_capabilities() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    println!("cargo:warning=🚀 Building Motiv for: {}", target_os);

    match target_os.as_str() {
        "macos" => {
            println!("cargo:warning=✅ macOS: Metal GPU acceleration ENABLED by default");
            #[cfg(feature = "coreml")]
            println!("cargo:warning=✅ CoreML acceleration ENABLED");
        }
        "windows" => {
            if cfg!(feature = "cuda") {
                println!("cargo:warning=✅ Windows: CUDA GPU acceleration ENABLED");
            } else if cfg!(feature = "vulkan") {
                println!("cargo:warning=✅ Windows: Vulkan GPU acceleration ENABLED");
            } else if cfg!(feature = "openblas") {
                println!("cargo:warning=✅ Windows: OpenBLAS CPU optimization ENABLED");
            } else {
                println!("cargo:warning=⚠️  Windows: Using CPU-only mode (no GPU or BLAS acceleration)");
                println!("cargo:warning=💡 For NVIDIA GPU: cargo build --release --features cuda");
                println!("cargo:warning=💡 For AMD/Intel GPU: cargo build --release --features vulkan");
                println!("cargo:warning=💡 For CPU optimization: cargo build --release --features openblas");

                // Try to detect NVIDIA GPU
                if which::which("nvidia-smi").is_ok() {
                    println!("cargo:warning=🎯 NVIDIA GPU detected! Consider rebuilding with --features cuda");
                }
            }
        }
        "linux" => {
            if cfg!(feature = "cuda") {
                println!("cargo:warning=✅ Linux: CUDA GPU acceleration ENABLED");
            } else if cfg!(feature = "vulkan") {
                println!("cargo:warning=✅ Linux: Vulkan GPU acceleration ENABLED");
            } else if cfg!(feature = "hipblas") {
                println!("cargo:warning=✅ Linux: AMD ROCm (HIP) acceleration ENABLED");
            } else if cfg!(feature = "openblas") {
                println!("cargo:warning=✅ Linux: OpenBLAS CPU optimization ENABLED");
            } else {
                println!("cargo:warning=⚠️  Linux: Using CPU-only mode (no GPU or BLAS acceleration)");
                println!("cargo:warning=💡 For NVIDIA GPU: cargo build --release --features cuda");
                println!("cargo:warning=💡 For AMD GPU: cargo build --release --features hipblas");
                println!("cargo:warning=💡 For other GPUs: cargo build --release --features vulkan");
                println!("cargo:warning=💡 For CPU optimization: cargo build --release --features openblas");

                // Try to detect NVIDIA GPU
                if which::which("nvidia-smi").is_ok() {
                    println!("cargo:warning=🎯 NVIDIA GPU detected! Consider rebuilding with --features cuda");
                }

                // Try to detect AMD GPU
                if which::which("rocm-smi").is_ok() {
                    println!("cargo:warning=🎯 AMD GPU detected! Consider rebuilding with --features hipblas");
                }
            }
        }
        _ => {
            println!("cargo:warning=ℹ️  Unknown platform: {}", target_os);
        }
    }

    // Performance guidance
    if !cfg!(feature = "cuda") && !cfg!(feature = "vulkan") && !cfg!(feature = "hipblas") && !cfg!(feature = "openblas") && target_os != "macos" {
        println!("cargo:warning=📊 Performance: CPU-only builds are significantly slower than GPU/BLAS builds");
        println!("cargo:warning=📚 See README.md for GPU/BLAS setup instructions");
    }
}

/// Motiv: merges the OAuth app registrations that get built into the app.
///
/// 1. `oauth_clients.json` (committed template, normally empty)
/// 2. `oauth_clients.local.json` (git-ignored; your real values)
/// 3. environment variables, for CI or one-off builds:
///    MOTIV_MS_CLIENT_ID, MOTIV_MS_TENANT, MOTIV_GOOGLE_CLIENT_ID, MOTIV_GOOGLE_CLIENT_SECRET
///
/// Later sources override earlier ones, per field. The result is written to
/// $OUT_DIR/oauth_clients.json and included by src/accounts/mod.rs.
fn embed_oauth_clients() {
    use serde_json::{json, Value};
    let read = |name: &str| -> Value {
        println!("cargo:rerun-if-changed={}", name);
        std::fs::read_to_string(name)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null)
    };
    let mut merged = json!({
        "microsoft": { "client_id": "", "tenant": "common" },
        "google": { "client_id": "", "client_secret": "" }
    });
    for source in [read("oauth_clients.json"), read("oauth_clients.local.json")] {
        for provider in ["microsoft", "google"] {
            if let Some(fields) = source.get(provider).and_then(|v| v.as_object()) {
                for (k, v) in fields {
                    if v.as_str().map_or(false, |s| !s.trim().is_empty()) {
                        merged[provider][k] = v.clone();
                    }
                }
            }
        }
    }
    for (var, provider, field) in [
        ("MOTIV_MS_CLIENT_ID", "microsoft", "client_id"),
        ("MOTIV_MS_TENANT", "microsoft", "tenant"),
        ("MOTIV_GOOGLE_CLIENT_ID", "google", "client_id"),
        ("MOTIV_GOOGLE_CLIENT_SECRET", "google", "client_secret"),
    ] {
        println!("cargo:rerun-if-env-changed={}", var);
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                merged[provider][field] = Value::String(v.trim().to_string());
            }
        }
    }
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("oauth_clients.json");
    std::fs::write(out, serde_json::to_string_pretty(&merged).unwrap()).unwrap();
    for provider in ["microsoft", "google"] {
        if merged[provider]["client_id"].as_str().map_or(true, |s| s.is_empty()) {
            println!("cargo:warning=Motiv: no built-in {} app registration (see docs/CALENDAR_SETUP.md)", provider);
        }
    }
}
