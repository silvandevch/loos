use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/loos-switch.rc");
    println!("cargo:rerun-if-changed=assets/loos-switch.manifest");
    println!("cargo:rerun-if-changed=assets/loos-switch.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let obj = out.join("loos-switch.res.o");
    let candidates = ["x86_64-w64-mingw32-windres", "windres"];
    let mut last = String::new();
    for tool in candidates {
        let status = Command::new(tool)
            .current_dir(PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets"))
            .args(["-O", "coff", "-o"])
            .arg(&obj)
            .arg("loos-switch.rc")
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("cargo:rustc-link-arg={}", obj.display());
                return;
            }
            Ok(s) => last = format!("{tool} exited with {s}"),
            Err(e) => last = format!("{tool}: {e}"),
        }
    }
    panic!("failed to compile Windows resources: {last}");
}
