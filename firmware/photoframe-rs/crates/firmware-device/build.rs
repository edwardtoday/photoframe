fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "espidf" {
        embuild::espidf::sysenv::output();
        let jpeg_lib = std::fs::canonicalize(
            "../../../photoframe-fw/managed_components/espressif__esp_new_jpeg/lib/esp32s3",
        )
        .expect("jpeg lib path");
        println!("cargo:rustc-link-search=native={}", jpeg_lib.display());
        println!("cargo:rustc-link-lib=static=esp_new_jpeg");
    }
}
