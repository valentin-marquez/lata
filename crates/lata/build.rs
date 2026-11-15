fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=lata.rc");
        println!("cargo:rerun-if-changed=../../assets/icons/lata.ico");
        embed_resource::compile("lata.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
