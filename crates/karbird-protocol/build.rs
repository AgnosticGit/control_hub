fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = "../../proto";
    println!("cargo:rerun-if-changed={proto_root}");
    let fds = protox::compile(["controlhub/v1/ingest.proto"], [proto_root])?;
    prost_build::Config::new().compile_fds(fds)?;
    Ok(())
}
