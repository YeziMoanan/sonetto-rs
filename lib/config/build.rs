use std::env;
use std::fs;
use std::path::Path;

mod excel_confgen;

//add tables to codegen/tables

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=excel_confgen");

    let custom_json_dir = env::var("JSON_DATA_DIR").ok();
    let json_dir = if let Some(custom_dir) = custom_json_dir.as_ref() {
        custom_dir.clone()
    } else {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        Path::new(&manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data//excel2json")
            .to_string_lossy()
            .to_string()
    };

    let output_dir = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("configs");
    let output_mod = output_dir.join("mod.rs");

    println!("cargo:rerun-if-changed={}", json_dir);

    if !Path::new(&json_dir).is_dir() {
        if custom_json_dir.is_none() && output_mod.is_file() {
            println!(
                "cargo:warning=JSON data directory {} is missing; using checked-in generated configs",
                json_dir
            );
            return;
        }

        eprintln!("JSON data directory does not exist: {}", json_dir);
        std::process::exit(1);
    }

    fs::create_dir_all(&output_dir).unwrap();

    if let Err(e) = excel_confgen::generate_rust_modules(&json_dir, &output_dir.to_string_lossy()) {
        eprintln!("Failed to generate Rust modules: {}", e);
        std::process::exit(1);
    }

    println!("Generated Rust modules in {:?}", output_dir);
}
