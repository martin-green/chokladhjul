fn main() {
    // Release builds get their version from the git tag via CHOKLADHJUL_VERSION (set by CI).
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CHOKLADHJUL_VERSION");
    let version = std::env::var("CHOKLADHJUL_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "dev".to_string());
    println!("cargo:rustc-env=APP_VERSION={}", version.trim());

    // Export the Optimus/PowerXpress hints from the executable so hybrid
    // graphics laptops (e.g. GeForce GT 630M + Intel HD 4000) pick the discrete GPU.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") && target.contains("msvc") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:NvOptimusEnablement,DATA");
        println!("cargo:rustc-link-arg-bins=/EXPORT:AmdPowerXpressRequestHighPerformance,DATA");
    }
}
