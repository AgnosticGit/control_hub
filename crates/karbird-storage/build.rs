fn main() {
    // `sqlx::migrate!` embeds the migrations at compile time; a new file must trigger a rebuild.
    println!("cargo:rerun-if-changed=../../infra/migrations/postgres");
}
