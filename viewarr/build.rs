use std::process::Command;

fn main() {
    // Get current timestamp
    let output = Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .expect("Failed to get timestamp");
    
    let timestamp = String::from_utf8_lossy(&output.stdout).trim().to_string();
    
    println!("cargo:rustc-env=BUILD_TIMESTAMP={}", timestamp);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/");
}
