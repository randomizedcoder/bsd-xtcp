use std::io::Result;

fn main() -> Result<()> {
    let mut prost_config = prost_build::Config::new();
    prost_config.btree_map(["."]);

    let descriptor_path =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("proto_descriptor.bin");

    prost_config.file_descriptor_set_path(&descriptor_path);
    prost_config.compile_protos(&["proto/tcp_stats.proto"], &["proto/"])?;

    let descriptor_set = std::fs::read(&descriptor_path)?;
    pbjson_build::Builder::new()
        .register_descriptors(&descriptor_set)?
        .build(&[".tcpstats_reader"])?;

    Ok(())
}
