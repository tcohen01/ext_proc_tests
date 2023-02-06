fn main() {
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .include_file("mod.rs")
        .out_dir("src/proto")
        .compile_well_known_types(true)
        .compile(
            &["third_party/envoy/api/envoy/service/ext_proc/v3/external_processor.proto"],
            &[
                "third_party/udpa",
                "third_party/xds",
                "third_party/protoc-gen-validate",
                "third_party/envoy/api",
            ],
        )
        .unwrap();
}
