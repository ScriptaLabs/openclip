fn main() {
    // The release channel is baked into the binary, so a changed value must rebuild it.
    println!("cargo:rerun-if-env-changed=OPENCLIP_REALTIME_URL");
    println!("cargo:rerun-if-env-changed=OPENCLIP_SUPABASE_ANON_KEY");
    tauri_build::build();
}
