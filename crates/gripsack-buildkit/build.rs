fn main() {
    println!("cargo:rerun-if-changed=src/llb/ops.proto");
    protobuf_codegen::Codegen::new()
        .pure()
        .include("src/llb")
        .input("src/llb/ops.proto")
        .cargo_out_dir("llb")
        .run_from_script();
}
