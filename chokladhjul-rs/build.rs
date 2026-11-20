fn main() {
    // Export the Optimus/PowerXpress hints from the executable so hybrid
    // graphics laptops (e.g. GeForce GT 630M + Intel HD 4000) pick the discrete GPU.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") && target.contains("msvc") {
        println!("cargo:rustc-link-arg-bins=/EXPORT:NvOptimusEnablement,DATA");
        println!("cargo:rustc-link-arg-bins=/EXPORT:AmdPowerXpressRequestHighPerformance,DATA");
    }
}
