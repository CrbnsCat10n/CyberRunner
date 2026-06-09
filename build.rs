use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/fonts/CyberRunnerFallbackCJK.otf");
    println!("cargo:rustc-check-cfg=cfg(cyber_runner_embedded_cjk_font)");

    if Path::new("assets/fonts/CyberRunnerFallbackCJK.otf").is_file() {
        println!("cargo:rustc-cfg=cyber_runner_embedded_cjk_font");
    }
}
