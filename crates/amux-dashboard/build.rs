fn main() {
    // New asset files must invalidate the embed, including Business UI builds.
    println!("cargo:rerun-if-changed=static");
}
