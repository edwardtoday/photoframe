fn main() {
    println!("cargo:rerun-if-env-changed=PHOTOFRAME_BOOTSTRAP_CONFIG_JSON");
    println!("cargo:rerun-if-env-changed=PHOTOFRAME_DEBUG_STAGE_BEACON");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "espidf" {
        embuild::espidf::sysenv::output();
        println!("cargo:rerun-if-changed=../../vendor/esp_new_jpeg/lib/esp32s3/libesp_new_jpeg.a");
        let jpeg_lib =
            std::fs::canonicalize("../../vendor/esp_new_jpeg/lib/esp32s3").expect("jpeg lib path");
        println!("cargo:rustc-link-search=native={}", jpeg_lib.display());
        println!("cargo:rustc-link-lib=static=esp_new_jpeg");
    }
}
