fn main() {
    // `embed_migrations!` reads SQL and metadata in a procedural macro. Cargo
    // does not otherwise know that changes to those external files must rebuild
    // this crate, which can leave stale migration metadata in cached binaries.
    println!("cargo:rerun-if-changed=migrations");
}
