fn main() {
    println!("cargo:rerun-if-changed=res/app.rc");
    println!("cargo:rerun-if-changed=res/app.manifest");
    println!("cargo:rerun-if-changed=res/app.ico");
    embed_resource::compile_for("res/app.rc", ["hyperlight-tray"], embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
